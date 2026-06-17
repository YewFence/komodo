use std::{
  fmt::Write,
  path::{Path, PathBuf},
};

use anyhow::{Context, anyhow};
use formatting::format_serror;
use komodo_client::entities::{EnvironmentVar, update::Log};
use shell_escape::unix::escape;

pub async fn write_dockerfile(
  build_path: &Path,
  dockerfile_path: &str,
  dockerfile: &str,
  logs: &mut Vec<Log>,
) {
  if let Err(e) = async {
    if dockerfile.is_empty() {
      return Err(anyhow!("UI Defined dockerfile is empty"));
    }

    let full_dockerfile_path = build_path
      .join(dockerfile_path)
      .components()
      .collect::<PathBuf>();

    mogh_secret_file::write_async(&full_dockerfile_path, dockerfile).await.with_context(|| {
      format!(
        "Failed to write dockerfile contents to {full_dockerfile_path:?}"
      )
    })?;

    logs.push(Log::simple(
      "Write Dockerfile",
      format!(
        "Dockerfile contents written to {full_dockerfile_path:?}"
      ),
    ));

    anyhow::Ok(())
  }.await {
    logs.push(Log::error("Write Dockerfile", format_serror(&e.into())));
  }
}

pub fn parse_build_args(build_args: &[EnvironmentVar]) -> String {
  build_args
    .iter()
    .map(|p| {
      // Escape the value for the shell: it must reach the docker
      // CLI byte-for-byte, with no shell interpretation in between.
      format!(
        " --build-arg {}={}",
        p.variable,
        escape(p.value.as_str().into())
      )
    })
    .collect::<Vec<_>>()
    .join("")
}

/// <https://docs.docker.com/build/building/secrets/#using-build-secrets>
pub async fn parse_secret_args(
  secret_args: &[EnvironmentVar],
  build_dir: &Path,
) -> anyhow::Result<String> {
  let mut res = String::new();
  for EnvironmentVar { variable, value } in secret_args {
    // Check edge cases
    if variable.is_empty() {
      return Err(anyhow!("secret variable cannot be empty string"));
    } else if variable.contains('=') {
      return Err(anyhow!(
        "invalid variable {variable}. variable cannot contain '='"
      ));
    }
    // Write the value to file to mount
    let path = build_dir.join(variable);
    mogh_secret_file::write_async(&path, value)
      .await
      .with_context(|| {
        format!(
          "Failed to write build secret {variable} to {}",
          path.display()
        )
      })?;
    // Extend the command
    write!(
      &mut res,
      " --secret id={variable},src={}",
      path.display()
    )
    .with_context(|| {
      format!(
        "Failed to format build secret arguments for {variable}"
      )
    })?;
  }
  Ok(res)
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Executes the given shell command with real `sh` and returns
  /// its stdout. The commands under test run through `sh -c` in
  /// production, so the test consumer must match.
  fn sh_stdout(command: &str) -> String {
    let output = std::process::Command::new("sh")
      .args(["-c", command])
      .output()
      .expect("failed to spawn sh");
    assert!(
      output.status.success(),
      "command failed: {command}\nstderr: {}",
      String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
  }

  /// Every build arg value must survive the round trip through
  /// `sh` byte-for-byte: values reach the docker CLI exactly as
  /// the user wrote them, with no shell interpretation in between.
  #[test]
  fn parse_build_args_values_survive_shell_round_trip() {
    let values = [
      "plain",
      "p@$$w0rd!",
      "$(id)",
      "pre`id`post",
      "say \"hi\"",
      "it's",
      "line1\nline2",
      "a # b",
      "trailing ",
      "*glob*",
      "C:\\path",
      "",
    ];

    let build_args = values
      .iter()
      .enumerate()
      .map(|(i, value)| EnvironmentVar {
        variable: format!("ARG_{i}"),
        value: value.to_string(),
      })
      .collect::<Vec<_>>();

    // printf '<%s>' prints every argument wrapped in <>,
    // so the exact argv the docker CLI would receive is observable.
    let command =
      format!("printf '<%s>'{}", parse_build_args(&build_args));

    let stdout = sh_stdout(&command);

    let mut expected = String::new();
    for (i, value) in values.iter().enumerate() {
      expected.push_str(&format!("<--build-arg><ARG_{i}={value}>"));
    }
    assert_eq!(stdout, expected);
  }
}
