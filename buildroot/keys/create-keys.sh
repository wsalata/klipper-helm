#!/bin/sh
# Creates the update signing key pair (run once).
#   release.cert.pem  public; goes into the image as the RAUC keyring and into git
#   release.key.pem   private; never commit it. For GitHub Actions, store its content
#                     in the repository secret RAUC_SIGNING_KEY.
# A new pair makes existing devices reject updates signed with it, so keep the key safe.
# No extendedKeyUsage: RAUC checks the S/MIME signing purpose, which codeSigning would exclude.
set -e
cd "$(dirname "$0")"
if [ -e release.key.pem ]; then
	echo "release.key.pem already exists, not overwriting it" >&2
	exit 1
fi
umask 077
openssl req -x509 -newkey rsa:4096 -sha256 -nodes -days 10950 \
	-subj "/O=Klipper Helm/CN=Klipper Helm update signing" \
	-addext "keyUsage=critical,digitalSignature" \
	-keyout release.key.pem -out release.cert.pem
chmod 644 release.cert.pem
echo "Created release.cert.pem and release.key.pem"
