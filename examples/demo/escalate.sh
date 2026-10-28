#!/bin/sh
set -eu
echo "escalation: triggered"
echo "alert=${alertname:-manual}"
exit 0
