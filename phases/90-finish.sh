#!/usr/bin/env bash
# Summary and the only point that waits for the user.

summary
echo
echo "Log: $LOG_FILE"
echo
read -rsn1 -p "Press o to open the log, any other key to finish " key </dev/tty
echo
if [[ "${key:-}" == "o" ]]; then
    "${PAGER:-less}" "$LOG_FILE" </dev/tty >/dev/tty
fi
