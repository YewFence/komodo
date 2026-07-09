use anyhow::anyhow;
use encoding::{Decode as _, Encode as _};
use mogh_pki::SpkiPublicKey;
use periphery_client::transport::{
  EncodedLoginMessage, LoginMessage, TransportMessage,
};
use std::time::Duration;

use crate::{
  auth::AUTH_TIMEOUT,
  websocket::{Websocket, WebsocketExt},
};

pub trait LoginWebsocketExt: WebsocketExt {
  fn send_login_error(
    &mut self,
    e: &anyhow::Error,
  ) -> impl Future<Output = anyhow::Result<()>> + Send {
    let message =
      TransportMessage::Login(EncodedLoginMessage::from(e.encode()));
    self.send_message(message)
  }

  fn recv_login_message(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<LoginMessage>> + Send {
    self.recv_login_message_with_timeout(AUTH_TIMEOUT)
  }

  fn recv_login_message_with_timeout(
    &mut self,
    timeout: Duration,
  ) -> impl Future<Output = anyhow::Result<LoginMessage>> + Send {
    async move {
      let TransportMessage::Login(message) =
        self.recv_message().with_timeout(timeout).await?
      else {
        return Err(anyhow!(
          "Expected Login message, got other message type"
        ));
      };
      message.decode()
    }
  }

  fn recv_login_success(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<()>> + Send {
    self.recv_login_success_with_timeout(AUTH_TIMEOUT)
  }

  fn recv_login_success_with_timeout(
    &mut self,
    timeout: Duration,
  ) -> impl Future<Output = anyhow::Result<()>> + Send {
    async move {
      let LoginMessage::Success =
        self.recv_login_message_with_timeout(timeout).await?
      else {
        return Err(anyhow!(
          "Expected Login Success message, got other message type"
        ));
      };
      Ok(())
    }
  }

  fn recv_login_nonce(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<[u8; 32]>> + Send {
    self.recv_login_nonce_with_timeout(AUTH_TIMEOUT)
  }

  fn recv_login_nonce_with_timeout(
    &mut self,
    timeout: Duration,
  ) -> impl Future<Output = anyhow::Result<[u8; 32]>> + Send {
    async move {
      let LoginMessage::Nonce(nonce) =
        self.recv_login_message_with_timeout(timeout).await?
      else {
        return Err(anyhow!(
          "Expected Login Nonce message, got other message type"
        ));
      };
      Ok(nonce)
    }
  }

  fn recv_login_handshake_bytes(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<Vec<u8>>> + Send {
    self.recv_login_handshake_bytes_with_timeout(AUTH_TIMEOUT)
  }

  fn recv_login_handshake_bytes_with_timeout(
    &mut self,
    timeout: Duration,
  ) -> impl Future<Output = anyhow::Result<Vec<u8>>> + Send {
    async move {
      let LoginMessage::Handshake(bytes) =
        self.recv_login_message_with_timeout(timeout).await?
      else {
        return Err(anyhow!(
          "Expected Login Handshake message, got other message type"
        ));
      };
      Ok(bytes)
    }
  }

  fn recv_login_onboarding_flow(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<bool>> + Send {
    self.recv_login_onboarding_flow_with_timeout(AUTH_TIMEOUT)
  }

  fn recv_login_onboarding_flow_with_timeout(
    &mut self,
    timeout: Duration,
  ) -> impl Future<Output = anyhow::Result<bool>> + Send {
    async move {
      let LoginMessage::OnboardingFlow(onboarding_flow) =
        self.recv_login_message_with_timeout(timeout).await?
      else {
        return Err(anyhow!(
          "Expected Login OnboardingFlow message, got other message type"
        ));
      };
      Ok(onboarding_flow)
    }
  }

  fn recv_login_public_key(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<SpkiPublicKey>> + Send {
    async {
      let LoginMessage::PublicKey(public_key) =
        self.recv_login_message().await?
      else {
        return Err(anyhow!(
          "Expected Login PublicKey message, got other message type"
        ));
      };
      Ok(public_key)
    }
  }

  fn recv_login_v1_passkey_flow(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<bool>> + Send {
    async {
      let LoginMessage::V1PasskeyFlow(v1_passkey_flow) =
        self.recv_login_message().await?
      else {
        return Err(anyhow!(
          "Expected Login V1PasskeyFlow message, got other message type"
        ));
      };
      Ok(v1_passkey_flow)
    }
  }

  fn recv_login_v1_passkey(
    &mut self,
  ) -> impl Future<Output = anyhow::Result<Vec<u8>>> + Send {
    async {
      let LoginMessage::V1Passkey(bytes) =
        self.recv_login_message().await?
      else {
        return Err(anyhow!(
          "Expected Login V1Passkey message, got other message type"
        ));
      };
      Ok(bytes)
    }
  }
}

impl<W: Websocket> LoginWebsocketExt for W {}
