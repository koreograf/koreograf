#!/bin/sh
set -eu
echo "eskalation: der healthcheck ist fehlgeschlagen"
echo "alert=${alertname:-manual}"
exit 0
