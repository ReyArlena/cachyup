#!/usr/bin/env bash
# All the long waits live here. Go and make tea.

heading "Pacman packages"
install_list pacman "$BOOTSTRAP_DIR/packages/pacman.txt"

heading "AUR packages"
if has_cmd yay; then
    install_list aur "$BOOTSTRAP_DIR/packages/aur.txt"
else
    status "AUR packages" FAILED "yay not installed"
fi

heading "Flatpaks"
if has_cmd flatpak; then
    step "Flathub remote" flathub_present flathub_add
    install_list flatpak "$BOOTSTRAP_DIR/packages/flatpak.txt"
else
    status "Flatpaks" FAILED "flatpak not installed"
fi

heading "Conditional packages"
run_scripts "$BOOTSTRAP_DIR/conditional"
