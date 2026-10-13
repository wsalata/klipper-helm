#!/bin/sh
# Renders boot-logo.png (the kernel's boot logo, shown until the panel starts)
# from the panel's logo and fonts. Needs ImageMagick 7 with SVG support.
# Black background: the kernel console's background is black.
set -e
cd "$(dirname "$0")"
UI=../../../ui
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

magick -background none -density 300 "$UI/icons/logo.svg" -resize 150x150 "$TMP/mark.png"
magick -background none -fill '#E8EEF6' -font "$UI/fonts/Saira-Medium.ttf" -pointsize 58 label:klipper "$TMP/word1.png"
magick -background none -fill '#2EC4B6' -font "$UI/fonts/Saira-SemiBold.ttf" -pointsize 58 label:helm "$TMP/word2.png"
magick "$TMP/word1.png" "$TMP/word2.png" -background none +append "$TMP/word.png"
magick "$TMP/mark.png" \( -size 1x22 xc:none \) "$TMP/word.png" -background none -gravity center -append \
	-background black -alpha remove -alpha off -gravity center -extent 440x290 +repage boot-logo.png
echo "boot-logo.png: $(magick identify -format '%wx%h, %k colours' boot-logo.png)"
