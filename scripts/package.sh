#!/usr/bin/env bash

set -euo pipefail

target="${RUST_TARGET:-}"
suffix="${RELEASE_SUFFIX:?RELEASE_SUFFIX is required}"
output_dir="${RELEASE_ARTIFACT_DIR:-dist}"

if [[ -n "${target}" ]]; then
  target_dir="target/${target}/release"
else
  target_dir="target/release"
fi

mkdir -p "${output_dir}"
install -m 755 "${target_dir}/km" "${output_dir}/km-${suffix}"
install -m 755 "${target_dir}/periphery" "${output_dir}/periphery-${suffix}"
strip "${output_dir}/km-${suffix}" "${output_dir}/periphery-${suffix}"
