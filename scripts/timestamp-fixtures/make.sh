#!/bin/sh
# Makes the timestamp tokens the tests of src-tauri/src/timestamp/ read (fixtures/): a test authority's,
# made here with openssl, good and broken in every way a token is checked, and DigiCert's own, fetched
# (`--digicert`, needs the internet: http://timestamp.digicert.com). The tokens timestamp the SHA-256 of
# "bullseye fixture"; the test authority's root is pinned in the tests only.
set -e
here=$(cd "$(dirname "$0")" && pwd)
out="$here/../../src-tauri/src/timestamp/fixtures"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"
mkdir -p "$out"
printf 'bullseye fixture' > data
openssl ts -query -data data -sha256 -cert -no_nonce -out query.tsq 2>/dev/null

if [ "$1" = "--digicert" ]; then
    curl -sSf -m 30 ${CURL_PROXY:+--proxytunnel -x "$CURL_PROXY"} -H 'Content-Type: application/timestamp-query' \
        --data-binary @query.tsq -o "$out/digicert.tsr" http://timestamp.digicert.com
    exit 0
fi

key() { openssl genrsa -out "$1.key" "${2:-2048}" 2>/dev/null; }
mkdir new && : > index.txt && echo 1000 > caserial
cat > ca.cnf <<CNF
[ ca ]
default_ca = d
[ d ]
database = $work/index.txt
new_certs_dir = $work/new
serial = $work/caserial
policy = p
default_md = sha256
unique_subject = no
copy_extensions = none
[ p ]
commonName = supplied
CNF
# cert NAME ISSUER FROM TO EXTENSIONS: a certificate for NAME.key, signed by ISSUER (or itself), valid
# FROM TO (YYMMDDHHMMSSZ).
cert() {
    printf '[ x ]\n%s\n' "$5" > "$1.ext"
    openssl req -new -key "$1.key" -subj "/CN=Bullseye test $1" -out "$1.csr" 2>/dev/null
    if [ "$2" = "self" ]; then sign="-selfsign -keyfile $1.key"; else sign="-cert $2.pem -keyfile $2.key"; fi
    openssl ca -batch -config ca.cnf -notext $sign -in "$1.csr" -out "$1.pem" -extfile "$1.ext" -extensions x \
        -startdate "$3" -enddate "$4" 2>/dev/null
}
ca='basicConstraints=critical,CA:TRUE
keyUsage=critical,keyCertSign,cRLSign'
tsa='basicConstraints=CA:FALSE
keyUsage=critical,digitalSignature
extendedKeyUsage=critical,timeStamping'
key root 4096; cert root self 200101000000Z 491231235959Z "$ca"
key inter; cert inter root 200101000000Z 491231235959Z "$ca"
key tsa; cert tsa inter 200101000000Z 491231235959Z "$tsa"
# Its timestamping use not marked critical, as RFC 3161 requires it to be.
key noeku; cert noeku inter 200101000000Z 491231235959Z 'basicConstraints=CA:FALSE
keyUsage=critical,digitalSignature
extendedKeyUsage=timeStamping'
key old; cert old inter 200101000000Z 210101000000Z "$tsa"
key stranger 2048; cert stranger self 200101000000Z 491231235959Z "$ca"
key lost; cert lost stranger 200101000000Z 491231235959Z "$tsa"
openssl rsa -in root.key -pubout -outform DER -out "$out/test-root.der" 2>/dev/null
# openssl signs no token with a signer that is not for timestamping, so its certificate alone is
# checked, beside one that is.
openssl x509 -in noeku.pem -outform DER -out "$out/not-for-timestamps.der"
openssl x509 -in tsa.pem -outform DER -out "$out/for-timestamps.der"
# For tests that make tokens of their own at a time they choose (`timestamp::tests::token`): the test
# authority's signer key and the certificate of its issuer.
openssl pkcs8 -topk8 -nocrypt -in tsa.key -outform DER -out "$out/test-signer.pk8"
openssl x509 -in inter.pem -outform DER -out "$out/test-issuer.der"

# reply NAME SIGNER CHAIN DIGEST: a token from SIGNER (its chain: CHAIN.pem), signed with DIGEST.
reply() {
    cat > ts.cnf <<CNF
[ tsa ]
default_tsa = tsa_config
[ tsa_config ]
serial = $work/serial
signer_cert = $work/$2.pem
certs = $work/$3.pem
signer_key = $work/$2.key
signer_digest = $4
default_policy = 1.2.3.4.1
digests = sha256
accuracy = secs:1
ess_cert_id_alg = sha256
CNF
    [ -f serial ] || echo 01 > serial
    openssl ts -reply -config ts.cnf -queryfile query.tsq -out "$out/$1.tsr" 2>/dev/null >/dev/null
}
cp inter.pem chain.pem
reply test tsa chain sha256
reply test-sha384 tsa chain sha384
reply test-sha512 tsa chain sha512
reply test-expired old chain sha256
cp stranger.pem lost-chain.pem
reply test-unknown-root lost lost-chain sha256
printf '' > none.pem
reply test-no-chain tsa none sha256
