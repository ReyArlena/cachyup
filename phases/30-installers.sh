#!/usr/bin/env bash
# Apps with their own install script (curl | bash style).

heading "Installers"
run_scripts "$BOOTSTRAP_DIR/installers"
