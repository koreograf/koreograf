#!/bin/sh
set -eu
echo "healthcheck ok"
echo "alert=${alertname:-manual}"
echo "instance=${instance:-localhost}"
