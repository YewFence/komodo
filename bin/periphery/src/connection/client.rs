use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use axum::http::{HeaderValue, StatusCode};
use periphery_client::{
  CONNECTION_RETRY_SECONDS, transport::LoginMessage,
};
use tracing::Instrument;
use transport::{
  auth::{
    AddressConnectionIdentifiers, ClientLoginFlow,
    ConnectionIdentifiers, LoginFlow, LoginFlowArgs,
  },
  fix_ws_address,
  websocket::{
    WebsocketExt, login::LoginWebsocketExt,
    tungstenite::TungsteniteWebsocket,
  },
};

use crate::{
  config::periphery_config,
  connection::core_public_keys,
  state::{core_connections, periphery_keys},
};

const RETRY_LOG_INTERVAL: Duration = Duration::from_secs(60);
const RETRY_LOG_EVERY: u64 = 12;

#[instrument("StartCoreConnection")]
pub async fn handler(
  address: &str,
) -> anyhow::Result<tokio::task::JoinHandle<anyhow::Result<()>>> {
  let config = periphery_config();
  let address = fix_ws_address(address);
  let identifiers = AddressConnectionIdentifiers::extract(&address)?;
  let query =
    format!("server={}", urlencoding::encode(&config.connect_as));
  let endpoint = format!("{address}/ws/periphery?{query}");

  info!("Initiating outbound connection to {endpoint}");

  let mut connection_errors = RetryLogState::default();
  let mut login_errors = RetryLogState::default();
  let mut onboarding_errors = RetryLogState::default();

  let core = identifiers.host().to_string();

  let channel = core_connections().get_or_insert_default(&core).await;

  let handle = tokio::spawn(async move {
    let mut receiver = channel.receiver()?;
    loop {
      let (mut socket, accept) = match connect_websocket(&endpoint)
        .await
      {
        Ok(res) => res,
        Err(e) => {
          if let Some(retry_count) =
            connection_errors.record_failure(Instant::now())
          {
            warn!(phase = "websocket connect", retry_count, "{e:#}");
            // If error transitions from login to connection,
            // reset these to see the next login / onboarding error.
            login_errors.reset();
            onboarding_errors.reset();
          }
          tokio::time::sleep(Duration::from_secs(
            CONNECTION_RETRY_SECONDS,
          ))
          .await;
          continue;
        }
      };

      // Receive whether to use Server connection flow vs Server onboarding flow.
      let onboarding_flow = match socket
        .recv_login_onboarding_flow_with_timeout(
          periphery_config().connection_auth_timeout_duration(),
        )
        .await
        .context("Failed to receive Login OnboardingFlow message")
      {
        Ok(onboarding_flow) => onboarding_flow,
        Err(e) => {
          if let Some(retry_count) =
            connection_errors.record_failure(Instant::now())
          {
            warn!(
              phase = "login onboarding flow receive",
              retry_count, "{e:#}"
            );
            // If error transitions from login to connection,
            // reset these to see the next login / onboarding error.
            login_errors.reset();
            onboarding_errors.reset();
          }
          tokio::time::sleep(Duration::from_secs(
            CONNECTION_RETRY_SECONDS,
          ))
          .await;
          continue;
        }
      };

      connection_errors.reset();

      debug!(
        host = identifiers.host(),
        query,
        sec_websocket_accept = accept.to_str().unwrap_or_default(),
        "[CORE AUTH] Zero trust identifiers"
      );

      let identifiers =
        identifiers.build(accept.as_bytes(), query.as_bytes());

      if onboarding_flow {
        if let Err(e) = handle_onboarding(socket, identifiers).await.map(|onboarding_key| if onboarding_key {
          Ok(())
        } else {
          Err(anyhow!("Server '{}' does not exist or is misconfigured, and no PERIPHERY_ONBOARDING_KEY is provided.", config.connect_as))
        }) {
          if let Some(retry_count) =
            onboarding_errors.record_failure(Instant::now())
          {
            error!(phase = "onboarding flow", retry_count, "{e:#}");
          }
          tokio::time::sleep(Duration::from_secs(
            CONNECTION_RETRY_SECONDS,
          ))
          .await;
          continue;
        } else {
          onboarding_errors.reset();
        };
      } else {
        let span = info_span!(
          "CoreLogin",
          address,
          direction = "PeripheryToCore",
        );
        let login = async {
          super::handle_login::<_, ClientLoginFlow>(
            &mut socket,
            identifiers,
            false,
          )
          .await
        }
        .instrument(span)
        .await;
        if let Err(e) = login {
          // Try using onboarding key to fix public key issue.
          let e = match handle_onboarding(socket, identifiers).await {
            // Should work on next reconnect
            Ok(true) => continue,
            // No onboarding key available, use original error.
            Ok(false) => e,
            // Onboarding key available but failed.
            Err(onboarding_error) => onboarding_error.context(format!(
              "Standard login failed before fallback onboarding | {e:#}"
            )),
          };
          if let Some(retry_count) =
            login_errors.record_failure(Instant::now())
          {
            warn!(
              phase = "standard login or fallback onboarding",
              retry_count, "Failed to login | {e:#}"
            );
          }
          tokio::time::sleep(Duration::from_secs(
            CONNECTION_RETRY_SECONDS,
          ))
          .await;
          continue;
        }

        login_errors.reset();

        super::handle_socket(
          socket,
          &core,
          &channel.sender,
          &mut receiver,
        )
        .await
      }
    }
  });

  Ok(handle)
}

#[instrument("OnboardingFlow", skip_all)]
async fn handle_onboarding(
  mut socket: TungsteniteWebsocket,
  identifiers: ConnectionIdentifiers<'_>,
) -> anyhow::Result<bool> {
  let config = periphery_config();
  let Some(onboarding_key) = config.onboarding_key.as_deref() else {
    return Ok(false);
  };

  // .with_context(|| format!("Server '{}' does not exist or is misconfigured, and no PERIPHERY_ONBOARDING_KEY is provided.", config.connect_as))?;

  ClientLoginFlow::login(LoginFlowArgs {
    private_key: onboarding_key,
    identifiers,
    public_key_validator: core_public_keys(),
    auth_timeout: config.connection_auth_timeout_duration(),
    socket: &mut socket,
    should_close: true,
  })
  .await
  .context("Onboarding failed")?;

  // Post onboarding login 1: Send public key
  socket
    .send_message(LoginMessage::PublicKey(
      periphery_keys().load().public.clone(),
    ))
    .await
    .context("Failed to send public key bytes")?;

  socket
    .recv_login_success_with_timeout(
      config.connection_auth_timeout_duration(),
    )
    .await
    .context("Failed to receive Server onboarding result")?;

  info!(
    "Server onboarding flow for '{}' successful ✅",
    config.connect_as
  );

  Ok(true)
}

async fn connect_websocket(
  url: &str,
) -> anyhow::Result<(TungsteniteWebsocket, HeaderValue)> {
  let config = periphery_config();
  connect_websocket_with_options(
    url,
    config.core_tls_insecure_skip_verify,
    config.outbound_connect_timeout_duration(),
    &config.connect_as,
  )
  .await
}

async fn connect_websocket_with_options(
  url: &str,
  tls_insecure_skip_verify: bool,
  timeout: Option<Duration>,
  connect_as: &str,
) -> anyhow::Result<(TungsteniteWebsocket, HeaderValue)> {
  let connect = TungsteniteWebsocket::connect_maybe_tls_insecure(
    url,
    tls_insecure_skip_verify,
  );
  let res = if let Some(timeout) = timeout {
    tokio::time::timeout(timeout, connect).await.with_context(
      || {
        format!(
          "Timed out after {} seconds connecting to Core websocket",
          timeout.as_secs()
        )
      },
    )?
  } else {
    connect.await
  };
  res.map_err(|e| match e.status {
    StatusCode::NOT_FOUND => anyhow!("404 Not Found: Server '{connect_as}' does not exist."),
    StatusCode::BAD_REQUEST => anyhow!("400 Bad Request: Server '{connect_as}' is disabled or configured to make Core → Periphery connection"),
    StatusCode::UNAUTHORIZED => anyhow!("401 Unauthorized: Only one Server connected as '{connect_as}' is allowed. Or the Core reverse proxy needs to forward host and websocket headers."),
    _ => e.error,
  })
}

#[derive(Default)]
struct RetryLogState {
  failures: u64,
  last_logged_at: Option<Instant>,
}

impl RetryLogState {
  fn record_failure(&mut self, now: Instant) -> Option<u64> {
    self.failures += 1;

    let should_log = self.failures == 1
      || self.failures.is_multiple_of(RETRY_LOG_EVERY)
      || self.last_logged_at.is_some_and(|last_logged_at| {
        now.duration_since(last_logged_at) >= RETRY_LOG_INTERVAL
      });

    if should_log {
      self.last_logged_at = Some(now);
      Some(self.failures)
    } else {
      None
    }
  }

  fn reset(&mut self) {
    self.failures = 0;
    self.last_logged_at = None;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn retry_log_state_logs_first_failure_and_periodic_repeats() {
    let mut state = RetryLogState::default();
    let start = Instant::now();

    assert_eq!(state.record_failure(start), Some(1));

    for offset in 1..11 {
      assert_eq!(
        state.record_failure(start + Duration::from_secs(offset)),
        None
      );
    }

    assert_eq!(
      state.record_failure(start + Duration::from_secs(11)),
      Some(12)
    );
  }

  #[test]
  fn retry_log_state_logs_after_interval_and_resets() {
    let mut state = RetryLogState::default();
    let start = Instant::now();

    assert_eq!(state.record_failure(start), Some(1));
    assert_eq!(
      state.record_failure(start + Duration::from_secs(30)),
      None
    );
    assert_eq!(
      state.record_failure(start + RETRY_LOG_INTERVAL),
      Some(3)
    );

    state.reset();

    assert_eq!(
      state.record_failure(start + RETRY_LOG_INTERVAL),
      Some(1)
    );
  }

  #[tokio::test]
  async fn outbound_connect_timeout_covers_websocket_upgrade() {
    let listener =
      tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stalled_server = tokio::spawn(async move {
      let (_socket, _) = listener.accept().await.unwrap();
      std::future::pending::<()>().await;
    });

    let result = connect_websocket_with_options(
      &format!("ws://{address}"),
      false,
      Some(Duration::from_secs(1)),
      "timeout-test",
    )
    .await;
    stalled_server.abort();
    let error = match result {
      Ok(_) => panic!("websocket upgrade unexpectedly completed"),
      Err(error) => error,
    };

    assert!(format!("{error:#}").contains(
      "Timed out after 1 seconds connecting to Core websocket"
    ));
  }
}
