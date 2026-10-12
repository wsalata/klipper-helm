################################################################################
#
# klipper-helm
#
################################################################################

# Built from this repository (the parent of the external tree)
KLIPPER_HELM_VERSION = local
KLIPPER_HELM_SITE = $(BR2_EXTERNAL_KLIPPER_HELM_PATH)/..
KLIPPER_HELM_SITE_METHOD = local
KLIPPER_HELM_DEPENDENCIES = host-pkgconf libinput libxkbcommon udev fontconfig

# Keep build products and the Buildroot trees out of the copied sources
KLIPPER_HELM_OVERRIDE_SRCDIR_RSYNC_EXCLUSIONS = \
	--exclude=/target --exclude=/.buildroot --exclude=/output --exclude=/.vscode

# KMS + software renderer only: no winit, X11, Wayland or GPU libraries
KLIPPER_HELM_CARGO_BUILD_OPTS = --no-default-features --features embedded

# Local sources skip Buildroot's download step, where cargo packages normally
# get their crates vendored, so vendor them here instead (needs network access).
define KLIPPER_HELM_VENDOR_CRATES
	mkdir -p $(@D)/.cargo $(BR_CARGO_HOME)
	cd $(@D) && $(HOST_MAKE_ENV) CARGO_HOME=$(BR_CARGO_HOME) \
		cargo vendor --locked VENDOR > $(@D)/.cargo/config.toml
endef
KLIPPER_HELM_PRE_BUILD_HOOKS += KLIPPER_HELM_VENDOR_CRATES

$(eval $(cargo-package))
