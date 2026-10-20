#!/usr/bin/env bash
#
# Generate a Software Bill of Materials (SBOM) in SPDX 2.3 JSON format from cargo metadata.
# Used in CI and release workflows to provide supply-chain transparency and compliance.
#
# Run locally with: ./scripts/generate-sbom.sh [output_path]

set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p target
OUTPUT="${1:-target/sbom.spdx.json}"

echo "Generating Software Bill of Materials (SPDX 2.3) -> ${OUTPUT}..."

cargo metadata --format-version 1 --locked | jq '
{
  "spdxVersion": "SPDX-2.3",
  "dataLicense": "CC0-1.0",
  "SPDXID": "SPDXRef-DOCUMENT",
  "name": "hephos",
  "documentNamespace": ("https://github.com/arthurlch/hephos/sbom/" + (now | tostring)),
  "creationInfo": {
    "creators": ["Tool: hephos-generate-sbom", "Organization: Hephos"],
    "created": (now | strftime("%Y-%m-%dT%H:%M:%SZ"))
  },
  "packages": [
    .packages[] | {
      "SPDXID": ("SPDXRef-Package-" + .name + "-" + .version),
      "name": .name,
      "versionInfo": .version,
      "licenseConcluded": (.license // "NOASSERTION"),
      "licenseDeclared": (.license // "NOASSERTION"),
      "downloadLocation": (.repository // "NOASSERTION"),
      "filesAnalyzed": false
    }
  ]
}' > "$OUTPUT"

echo "✓ SBOM generated successfully (${OUTPUT}): $(wc -l < "$OUTPUT" | tr -d ' ') lines, $(jq '.packages | length' "$OUTPUT") packages"
