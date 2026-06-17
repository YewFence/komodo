use std::path::{Path, PathBuf};

use anyhow::Context;
use formatting::format_serror;
use komodo_client::entities::update::Log;

/// Writes the raw environment text to the env file verbatim
/// (only ensuring a trailing newline).
///
/// The content is passed through untouched: the consumer
/// (docker compose, or a shell sourcing the file) is the sole
/// judge of its syntax.
///
/// If the environment was written and needs to be passed to the
/// compose command, will return the env file PathBuf.
/// Should ensure all logs are successful after calling.
pub async fn write_env_file(
  environment: &str,
  folder: &Path,
  env_file_path: &str,
  logs: &mut Vec<Log>,
) -> Option<PathBuf> {
  let env_file_path =
    folder.join(env_file_path).components().collect::<PathBuf>();

  if environment.is_empty() {
    // Still want to return Some(env_file_path) if the path
    // already exists on the host and is a file.
    // This is for "Files on Server" mode when user writes the env file themself.
    if env_file_path.is_file() {
      return Some(env_file_path);
    }
    return None;
  }

  let contents = if environment.ends_with('\n') {
    environment.to_string()
  } else {
    format!("{environment}\n")
  };

  if let Err(e) =
    mogh_secret_file::write_async(&env_file_path, contents)
      .await
      .with_context(|| {
        format!(
          "Failed to write environment file to {env_file_path:?}"
        )
      })
  {
    logs.push(Log::error(
      "Write Environment File",
      format_serror(&e.into()),
    ));
    return None;
  }

  logs.push(Log::simple(
    "Write Environment File",
    format!("Environment file written to {env_file_path:?}"),
  ));

  Some(env_file_path)
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Raw passthrough: the written file bytes must equal the input
  /// text byte-for-byte, no matter what the content looks like.
  /// The consumer (docker compose, or a shell sourcing the file)
  /// is the sole judge of syntax.
  #[tokio::test]
  async fn write_env_file_passes_content_through_untouched() {
    let inputs = [
      "FOO=bar\n",
      "FOO=bar", // trailing newline added
      "KEY=\"value # not comment\"\nPASS=p@$$w0rd!\n",
      "MULTILINE='line 1\nline 2'\nESCAPED=\"a\\nb\"\n",
      "# only a comment\n",
      "THIS IS not even valid dotenv ;;;\n",
      "A=1\r\nB=2\r\n", // CRLF preserved
    ];

    let dir = std::env::temp_dir()
      .join(format!("env-raw-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    for (i, input) in inputs.iter().enumerate() {
      let mut logs = Vec::new();
      let path =
        write_env_file(input, &dir, &format!(".env.{i}"), &mut logs)
          .await
          .unwrap_or_else(|| {
            panic!("input {i} should produce a file: {logs:?}")
          });
      let written = std::fs::read_to_string(&path).unwrap();
      let expected = if input.ends_with('\n') {
        input.to_string()
      } else {
        format!("{input}\n")
      };
      assert_eq!(
        written, expected,
        "input {i} must pass through byte-for-byte"
      );
    }

    std::fs::remove_dir_all(&dir).ok();
  }
}
