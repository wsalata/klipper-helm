################################################################################
#
# odroidxu4-blobs
#
################################################################################

# Hardkernel's U-Boot tree for the XU4; only the signed blobs in sd_fuse/ are used
ODROIDXU4_BLOBS_VERSION = 88af53fbcef8386cb4d5f04c19f4b2bcb69e90ca
ODROIDXU4_BLOBS_SITE = $(call github,hardkernel,u-boot,$(ODROIDXU4_BLOBS_VERSION))
ODROIDXU4_BLOBS_LICENSE = PROPRIETARY
ODROIDXU4_BLOBS_REDISTRIBUTE = NO
ODROIDXU4_BLOBS_INSTALL_TARGET = NO
ODROIDXU4_BLOBS_INSTALL_IMAGES = YES

ODROIDXU4_BLOBS_FILES = \
	bl1.bin.hardkernel \
	bl2.bin.hardkernel.720k_uboot \
	tzsw.bin.hardkernel

define ODROIDXU4_BLOBS_INSTALL_IMAGES_CMDS
	$(foreach f,$(ODROIDXU4_BLOBS_FILES), \
		$(INSTALL) -D -m 0644 $(@D)/sd_fuse/$(f) $(BINARIES_DIR)/$(f)
	)
endef

$(eval $(generic-package))
