# Builds the embedded panel image with Buildroot (see buildroot/README.md).
#   make image          download Buildroot if needed, configure and build sdcard.img
#   make menuconfig     change the Buildroot configuration
#   make panel-rebuild  rebuild only the panel after code changes, then the image

BR_VERSION := 2026.08
BR_SHA256  := 87aaca4164ea9d5c8085854953018263f7963f07c22e73a2a2185cc98c581c34
BR_DIR     := .buildroot/buildroot-$(BR_VERSION)
BR_OUT     := $(CURDIR)/.buildroot/output
BR_DEFCONFIG := klipper_helm_odroidxu4_defconfig
BR_MAKE    := $(MAKE) -C $(BR_DIR) O=$(BR_OUT) BR2_EXTERNAL=$(CURDIR)/buildroot

.PHONY: image menuconfig panel-rebuild clean-image

image: $(BR_OUT)/.config
	$(BR_MAKE)
	@echo "Image: $(BR_OUT)/images/sdcard.img"

$(BR_OUT)/.config: $(BR_DIR)/Makefile buildroot/configs/$(BR_DEFCONFIG)
	$(BR_MAKE) $(BR_DEFCONFIG)

$(BR_DIR)/Makefile:
	mkdir -p .buildroot
	curl -fL -o .buildroot/buildroot-$(BR_VERSION).tar.xz \
		https://buildroot.org/downloads/buildroot-$(BR_VERSION).tar.xz
	echo "$(BR_SHA256)  .buildroot/buildroot-$(BR_VERSION).tar.xz" | sha256sum -c -
	tar -xJf .buildroot/buildroot-$(BR_VERSION).tar.xz -C .buildroot
	touch $@

menuconfig: $(BR_OUT)/.config
	$(BR_MAKE) menuconfig

panel-rebuild: $(BR_OUT)/.config
	$(BR_MAKE) klipper-helm-dirclean
	$(BR_MAKE)

clean-image:
	rm -rf $(BR_OUT)
