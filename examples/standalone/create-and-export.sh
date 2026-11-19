#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/../.." && pwd)

if [[ -n "${AITD:-}" ]]; then
    aitd=$AITD
elif [[ -x "$repo_root/target/release/aitd" ]]; then
    aitd="$repo_root/target/release/aitd"
elif [[ -x "$repo_root/target/debug/aitd" ]]; then
    aitd="$repo_root/target/debug/aitd"
else
    printf 'Build aitd first or set AITD to the CLI executable.\n' >&2
    exit 1
fi

output_dir=$(mktemp -d "${TMPDIR:-/tmp}/aitd-standalone-example.XXXXXX")
cleanup() {
    "$aitd" project close --discard >/dev/null 2>&1 || true
    rm -rf -- "$output_dir"
}
trap cleanup EXIT

"$aitd" project new
"$aitd" run --file "$script_dir/resources/01-objects.aitcmd"
"$aitd" run --file "$script_dir/resources/02-layout.aitcmd"
"$aitd" validate
"$aitd" project save "$output_dir/demo.aitp"

mkdir -p "$output_dir/first" "$output_dir/second"
"$aitd" export bundle \
    --iop "$output_dir/first/demo.iop" \
    --header "$output_dir/first/object_ids.h"
"$aitd" export bundle \
    --iop "$output_dir/second/demo.iop" \
    --header "$output_dir/second/object_ids.h"

cmp "$output_dir/first/demo.iop" "$output_dir/second/demo.iop"
cmp "$output_dir/first/object_ids.h" "$output_dir/second/object_ids.h"
printf 'Project validated and saved; repeated IOP and object-ID header exports match.\n'
