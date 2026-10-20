#!/usr/bin/env bash
# Apps with their own install script (curl | bash style). See installers/README.

heading "Installers"
run_scripts "$BOOTSTRAP_DIR/installers"
