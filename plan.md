# Periphery outbound reconnect fix plan

## Confirmed behavior

Periphery outbound mode is intended to reconnect forever. The reconnect loop is in `bin/periphery/src/connection/client.rs`, and failures return to the loop after `CONNECTION_RETRY_SECONDS`, currently 5 seconds.

The observed log

```text
Failed to login | [Client] Failed to get handshake_m2: Decoded error message over Core-Periphery communication channel: [Server] Failed to get handshake_m1: Timed out waiting for message.: deadline has elapsed
```

means Core accepted the websocket upgrade, started `ServerLoginFlow`, sent the nonce, and then timed out waiting for Periphery's `handshake_m1`.

The relevant source path is:

1. Periphery waits for `handshake_m2` in `lib/transport/src/auth.rs`.
2. Core waits for `handshake_m1` in `lib/transport/src/auth.rs`.
3. Core sends the server-side error with `send_login_error`.
4. Periphery decodes that error in `lib/encoding/src/response.rs`.

The later log

```text
Failed to login | [Client] Failed to receive Login Success message: Timed out waiting for message.: deadline has elapsed
```

means Periphery got through `handshake_m2`, sent `handshake_m3`, and then timed out waiting for Core's login success message.

## Root causes

The login handshake timeout is too short for weak network conditions. `AUTH_TIMEOUT` is currently 2 seconds in `lib/transport/src/auth.rs`, and all login message receives go through `recv_login_message().with_timeout(AUTH_TIMEOUT)` in `lib/transport/src/websocket/login.rs`.

The outbound websocket connection attempt has no explicit timeout. `bin/periphery/src/connection/client.rs` calls `connect_websocket`, which calls `TungsteniteWebsocket::connect_maybe_tls_insecure`; the underlying `connect_async` is not wrapped in `tokio::time::timeout`.

The reconnect loop suppresses repeated logs for the same failure class. `already_logged_login_error`, `already_logged_connection_error`, and `already_logged_onboarding_error` can make an active retry loop look silent after the first repeated failure.

## Fix goals

Make weak network reconnect behavior bounded, observable, and configurable.

Do not change the authentication protocol or weaken public-key validation.

Keep defaults conservative enough for normal deployments, but no longer fragile on slow Tailscale or relay paths.

## Proposed changes

1. Add a configurable login timeout.

   Replace the hard-coded `AUTH_TIMEOUT` usage with a configurable duration. A practical default is 10 seconds. If the config plumbing is too broad for one patch, first raise the constant from 2 seconds to 10 seconds and leave a follow-up to make it configurable.

2. Add an explicit outbound connect timeout.

   Wrap `connect_websocket(&endpoint)` in `bin/periphery/src/connection/client.rs` with `tokio::time::timeout`. A practical default is 15 seconds. On timeout, return a normal connection error so the existing retry loop sleeps 5 seconds and retries.

3. Add periodic retry visibility.

   Keep log de-duplication, but add counters and emit a warning every N repeated failures or after a fixed interval such as 60 seconds. The warning should include the current failure phase and retry count.

4. Improve error context.

   When login fails, log whether the failure happened before onboarding-flow receive, during standard login, during fallback onboarding, or after websocket connect. This avoids needing Core logs to identify the local state machine phase.

5. Add regression tests where practical.

   Unit-test the retry logging helper if extracted. For connection timeouts, prefer a small async test around a helper function rather than a full websocket integration test. If the project does not have a good test harness for this area, document manual verification steps in the pull request instead.

## Implementation order

1. Introduce duration constants or config fields for login and outbound connect timeouts.
2. Wrap outbound connect with timeout.
3. Raise or configure login receive timeout.
4. Add periodic retry logging.
5. Run focused formatting and checks.

## Manual verification

Run Periphery in outbound mode against Core.

Block or delay traffic between Periphery and Core and verify:

1. Periphery retries after connection timeout instead of becoming silent forever.
2. Repeated login failures produce periodic logs.
3. A recovered connection logs `Logged in to Komodo Core ... websocket as Server ...`.
4. Existing successful handshakes still work with public-key validation enabled.

## Risk notes

Increasing the login timeout means bad or malicious websocket clients can occupy a login task longer. This is acceptable at 10 seconds for weak-network tolerance, but Core should still rely on normal network-level rate limiting or reverse proxy limits for public deployments.

Adding a connect timeout changes only outbound Periphery behavior and should not affect Core-to-Periphery mode.
