use std::{sync::Arc, time::Duration};

use anyhow::{Context, ensure};
use axum::{
  Router,
  extract::{State, WebSocketUpgrade},
  http::HeaderMap,
  response::Response,
  routing::get,
};
use bytes::Bytes;
use mogh_pki::{EncodedKeyPair, PkiKind};
use periphery_client::transport::LoginMessage;
use tokio::sync::{Mutex, oneshot};
use transport::{
  auth::{
    AddressConnectionIdentifiers, ClientLoginFlow,
    HeaderConnectionIdentifiers, LoginFlow, LoginFlowArgs,
    PublicKeyValidator, ServerLoginFlow,
  },
  timeout::MaybeWithTimeout,
  websocket::{
    Websocket, WebsocketExt, WebsocketMessage, WebsocketReceiver,
    WebsocketSender, axum::AxumWebsocket, login::LoginWebsocketExt,
    tungstenite::TungsteniteWebsocket,
  },
};

const QUERY: &str = "server=auth-timeout-test";

#[derive(Clone)]
struct ExpectedPublicKey(Arc<str>);

impl PublicKeyValidator for ExpectedPublicKey {
  type ValidationResult = ();

  async fn validate(&self, public_key: String) -> anyhow::Result<()> {
    ensure!(
      public_key == self.0.as_ref(),
      "received unexpected public key"
    );
    Ok(())
  }
}

struct DelayedSendWebsocket<W> {
  inner: W,
  delay: Duration,
}

impl<W: Websocket> Websocket for DelayedSendWebsocket<W> {
  fn split(self) -> (impl WebsocketSender, impl WebsocketReceiver) {
    self.inner.split()
  }

  async fn send(&mut self, bytes: Bytes) -> anyhow::Result<()> {
    tokio::time::sleep(self.delay).await;
    self.inner.send(bytes).await
  }

  async fn close(&mut self) -> anyhow::Result<()> {
    self.inner.close().await
  }

  fn recv_inner(
    &mut self,
  ) -> MaybeWithTimeout<
    impl Future<Output = anyhow::Result<WebsocketMessage>> + Send,
  > {
    self.inner.recv_inner()
  }
}

#[derive(Clone)]
struct ServerState {
  private_key: Arc<str>,
  expected_client_public_key: Arc<str>,
  auth_timeout: Duration,
  result: Arc<Mutex<Option<oneshot::Sender<anyhow::Result<()>>>>>,
}

async fn server_websocket(
  State(state): State<ServerState>,
  mut headers: HeaderMap,
  ws: WebSocketUpgrade,
) -> Response {
  let identifiers =
    HeaderConnectionIdentifiers::extract(&mut headers)
      .expect("failed to extract server connection identifiers");

  ws.on_upgrade(move |socket| async move {
    let mut socket = AxumWebsocket(socket);
    let result = async {
      socket
        .send_message(LoginMessage::OnboardingFlow(false))
        .await
        .context("failed to send onboarding flow")?;
      ServerLoginFlow::login(LoginFlowArgs {
        identifiers: identifiers.build(QUERY.as_bytes()),
        private_key: &state.private_key,
        public_key_validator: ExpectedPublicKey(
          state.expected_client_public_key.clone(),
        ),
        auth_timeout: state.auth_timeout,
        should_close: false,
        socket: &mut socket,
      })
      .await
    }
    .await;

    if let Some(result_sender) = state.result.lock().await.take() {
      let _ = result_sender.send(result);
    }
  })
}

async fn run_login(
  client_send_delay: Duration,
  client_auth_timeout: Duration,
  server_auth_timeout: Duration,
) -> (anyhow::Result<()>, anyhow::Result<()>) {
  let server_keys =
    EncodedKeyPair::generate(PkiKind::Mutual).unwrap();
  let client_keys =
    EncodedKeyPair::generate(PkiKind::Mutual).unwrap();
  let (result_sender, result_receiver) = oneshot::channel();

  let state = ServerState {
    private_key: Arc::from(server_keys.private()),
    expected_client_public_key: Arc::from(client_keys.public()),
    auth_timeout: server_auth_timeout,
    result: Arc::new(Mutex::new(Some(result_sender))),
  };
  let app = Router::new()
    .route("/ws/periphery", get(server_websocket))
    .with_state(state);
  let listener =
    tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let server = tokio::spawn(async move {
    axum::serve(listener, app).await.unwrap();
  });

  let endpoint = format!("ws://{address}/ws/periphery?{QUERY}");
  let address_identifiers =
    AddressConnectionIdentifiers::extract(&endpoint).unwrap();
  let (socket, accept) =
    TungsteniteWebsocket::connect(&endpoint).await.unwrap();
  let mut socket = DelayedSendWebsocket {
    inner: socket,
    delay: client_send_delay,
  };
  let client_result = async {
    let onboarding_flow = socket
      .recv_login_onboarding_flow_with_timeout(client_auth_timeout)
      .await?;
    ensure!(!onboarding_flow, "unexpected onboarding flow");

    ClientLoginFlow::login(LoginFlowArgs {
      identifiers: address_identifiers
        .build(accept.as_bytes(), QUERY.as_bytes()),
      private_key: client_keys.private(),
      public_key_validator: ExpectedPublicKey(Arc::from(
        server_keys.public(),
      )),
      auth_timeout: client_auth_timeout,
      should_close: false,
      socket: &mut socket,
    })
    .await
  }
  .await;
  let server_result = result_receiver
    .await
    .context("server did not report login result")
    .and_then(|result| result);
  server.abort();

  (client_result, server_result)
}

#[tokio::test]
async fn delayed_auth_succeeds_when_both_sides_allow_it() {
  let (client_result, server_result) = run_login(
    Duration::from_millis(200),
    Duration::from_secs(2),
    Duration::from_secs(2),
  )
  .await;

  client_result.unwrap();
  server_result.unwrap();
}

#[tokio::test]
async fn server_timeout_covers_waiting_for_handshake_m1() {
  let (client_result, server_result) = run_login(
    Duration::from_millis(250),
    Duration::from_secs(2),
    Duration::from_millis(50),
  )
  .await;

  let server_error = server_result.unwrap_err();
  assert!(
    format!("{server_error:#}").contains(
      "[Server] Failed to get handshake_m1: Timed out waiting for message"
    )
  );
  assert!(client_result.is_err());
}
