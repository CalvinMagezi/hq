#!/usr/bin/env bash
# Writes a small demo Vault with invented names, so recordings never show a real person, host or
# note. Run it into an empty folder, then start a throwaway HQ against it:
#   scripts/media/seed-demo-vault.sh /tmp/hq-demo-vault
#   HQ_VAULT_PATH=/tmp/hq-demo-vault HQ_CONFIG_PATH=/tmp/hq-demo-config.yaml hq web --json
set -euo pipefail
dir="${1:?usage: seed-demo-vault.sh <empty folder>}"
if [ -e "$dir" ] && [ -n "$(ls -A "$dir" 2>/dev/null)" ]; then
  echo "$dir is not empty; refusing to write into it" >&2
  exit 1
fi
mkdir -p "$dir/Notebooks/Garden" "$dir/Notebooks/Home"
cat > "$dir/Notebooks/Garden/Garden planner.md" <<'NOTE'
---
title: Garden planner
tags: [garden, plan]
---

# Garden planner

A small web app that tells you what to plant this week, from your city's first and last frost dates.

## Next steps

- Pull frost dates for the city from the open weather data.
- Show a week view with what to sow, transplant and harvest.
- Send a reminder the evening before a sowing day.
NOTE
cat > "$dir/Notebooks/Garden/Seed list.md" <<'NOTE'
# Seed list

Tomatoes, courgettes, runner beans, basil. Sow the beans after the last frost.
NOTE
cat > "$dir/Notebooks/Home/Bike service.md" <<'NOTE'
# Bike service

Replace the brake pads and the chain. Book the shop for Saturday morning.
NOTE
cat > "$dir/Notebooks/Home/Reading list.md" <<'NOTE'
# Reading list

A book on composting, a field guide to garden birds, a short history of tea.
NOTE
echo "Seeded $dir with four demo notes."
