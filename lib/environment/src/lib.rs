use std::path::{Path, PathBuf};

use anyhow::Context;
use formatting::format_serror;
use komodo_client::entities::{EnvironmentVar, update::Log};

/// If the environment was written and needs to be passed to the compose command,
/// will return the env file PathBuf.
/// Should ensure all logs are successful after calling.
pub async fn write_env_file(
  environment: &[EnvironmentVar],
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

  let contents = environment
    .iter()
    .map(|env| {
      format!("{}={}", env.variable, encode_dotenv_value(&env.value))
    })
    .collect::<Vec<_>>()
    .join("\n");

  let contents = if contents.is_empty() || contents.ends_with('\n') {
    contents
  } else {
    contents + "\n"
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

fn encode_dotenv_value(value: &str) -> String {
  if value.is_empty()
    || value.starts_with(char::is_whitespace)
    || value.ends_with(char::is_whitespace)
    || value.contains(['\n', '\r', '#', '\'', '\\'])
  {
    format!("'{}'", value.replace('\'', "'\\''"))
  } else {
    value.to_string()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn parse_value(encoded: &str) -> String {
    let env = format!("KEY={encoded}\n");
    dotenvy::from_read_iter(env.as_bytes())
      .next()
      .unwrap()
      .unwrap()
      .1
  }

  #[test]
  fn encodes_plain_dotenv_values_without_quotes() {
    assert_eq!(encode_dotenv_value("plain"), "plain");
    assert_eq!(parse_value(&encode_dotenv_value("plain")), "plain");
  }

  #[test]
  fn encodes_dotenv_values_that_need_quotes() {
    for value in [
      "",
      "line 1\nline 2",
      "it's quoted",
      "value # comment",
      " leading",
      "trailing ",
    ] {
      assert_eq!(parse_value(&encode_dotenv_value(value)), value);
    }
  }
}
