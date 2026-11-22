#!/usr/bin/env python3
# Usage: make-fixtures.py
# Writes the signed and encrypted test messages, keys and certificates of
# tests/crypto.rs into this folder, made by GnuPG, gpgsm and OpenSSL.
import os
import pathlib
import shutil
import subprocess
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
WORK = pathlib.Path(tempfile.mkdtemp(prefix="mf", dir="/tmp"))
PEOPLE = ["alice", "bob", "carol", "dave"]


def run(*command, data=None):
    return subprocess.run(command, input=data, capture_output=True, check=True).stdout


def gpgsm(home, *arguments, data=None):
    empty, writer = os.pipe()
    os.close(writer)
    command = ["gpgsm", "--homedir", home, "--batch", "--pinentry-mode", "loopback",
               "--passphrase-fd", str(empty), *arguments]
    try:
        return subprocess.run(command, input=data, capture_output=True, check=True,
                              pass_fds=(empty,)).stdout
    finally:
        os.close(empty)


def crlf(text):
    return text.replace("\r\n", "\n").replace("\n", "\r\n").encode()


def home(name):
    folder = WORK / name
    folder.mkdir(mode=0o700, exist_ok=True)
    return str(folder)


def gpg(name, *arguments, data=None):
    return run(
        "gpg", "--homedir", home(name), "--batch", "--yes", "--pinentry-mode",
        "loopback", "--passphrase", "", "--trust-model", "always", *arguments, data=data,
    )


def address(name):
    return f"{name}@bande.test"


def save(name, data):
    (HERE / name).write_bytes(data if isinstance(data, bytes) else data.encode())


def headers(sender, subject, content_type, extra=""):
    return (
        f"From: {sender.title()} <{address(sender)}>\nTo: Alice <alice@bande.test>\n"
        f"Subject: {subject}\nDate: Sun, 22 Nov 2026 09:00:00 +0100\n"
        f"Message-ID: <{subject.replace(' ', '-').lower()}@bande.test>\nMIME-Version: 1.0\n"
        f"{content_type if content_type.startswith('Content-') else 'Content-Type: ' + content_type}\n{extra}\n"
    )


def pgp_signed(signer, sender, subject, text):
    part = crlf(f"Content-Type: text/plain; charset=utf-8\n\n{text}\n")
    signature = gpg(signer, "--armor", "--detach-sign", "--local-user", address(signer), data=part)
    body = (
        b"--sig\r\n" + part + b"\r\n--sig\r\n"
        + crlf("Content-Type: application/pgp-signature; name=signature.asc\n\n")
        + signature.replace(b"\n", b"\r\n") + b"\r\n--sig--\r\n"
    )
    content_type = 'multipart/signed; boundary="sig"; micalg=pgp-sha256; protocol="application/pgp-signature"'
    return crlf(headers(sender, subject, content_type)) + body


def openpgp():
    for person in PEOPLE:
        gpg(person, "--quick-generate-key", f"{person.title()} <{address(person)}>", "future-default", "default", "never")
    for person in PEOPLE:
        exported = gpg(person, "--armor", "--export", address(person))
        if person == "carol":
            save("carol.pgp", exported)
        for other in PEOPLE:
            if other != person and person != "dave":
                gpg(other, "--import", data=exported)
    save("alice-secret.pgp", gpg("alice", "--armor", "--export-secret-keys", address("alice")))
    save("pgp-verified.eml", pgp_signed("alice", "alice", "Signed by Alice", "Alice signed this."))
    signed = pgp_signed("carol", "carol", "Signed by Carol", "Carol signed this.")
    save("pgp-unverified.eml", signed)
    save("pgp-tampered.eml", signed.replace(b"Carol signed this.", b"Carol signed that."))
    save("pgp-unknown.eml", pgp_signed("dave", "dave", "Signed by Dave", "Dave signed this."))
    save("pgp-wrong-sender.eml", pgp_signed("carol", "alice", "Signed for Alice", "Not Alice."))
    inner = crlf(
        'Content-Type: text/plain; charset=utf-8; protected-headers="v1"\n'
        "Subject: The real subject\n\nOnly Alice can read this.\n"
    )
    encrypted = gpg("carol", "--armor", "--sign", "--encrypt", "--local-user", address("carol"),
                    "--recipient", address("alice"), data=inner)
    save("pgp-encrypted.eml", pgp_encrypted("carol", "...", encrypted))
    to_bob = gpg("carol", "--armor", "--encrypt", "--recipient", address("bob"), data=inner)
    save("pgp-wrong-key.eml", pgp_encrypted("carol", "For Bob", to_bob))
    clear = gpg("carol", "--clearsign", "--local-user", address("carol"), data=b"Carol signed this inline.\n")
    save("pgp-inline.eml", crlf(headers("carol", "Inline", "text/plain; charset=utf-8")) + clear.replace(b"\n", b"\r\n"))
    partial = pgp_signed("carol", "carol", "Part", "Only this part is signed.")
    signed_part = partial.split(b"\r\n\r\n", 1)[1]
    mixed = (
        crlf(headers("carol", "Partly signed", 'multipart/mixed; boundary="mix"'))
        + crlf("--mix\nContent-Type: text/plain\n\nThis text is not signed.\n--mix\n")
        + crlf('Content-Type: multipart/signed; boundary="sig"; micalg=pgp-sha256; protocol="application/pgp-signature"\n\n')
        + signed_part + crlf("\n--mix--\n")
    )
    save("pgp-partial.eml", mixed)


def pgp_encrypted(sender, subject, armored):
    content_type = 'multipart/encrypted; boundary="enc"; protocol="application/pgp-encrypted"'
    return crlf(
        headers(sender, subject, content_type)
        + "--enc\nContent-Type: application/pgp-encrypted\n\nVersion: 1\n\n--enc\n"
        + "Content-Type: application/octet-stream; name=encrypted.asc\n\n"
        + armored.decode() + "\n--enc--\n"
    )


def openssl(*arguments, data=None):
    return run("openssl", *arguments, data=data)


def certificate(name, issuer, email):
    key, csr, cert = (str(WORK / f"{name}.{kind}") for kind in ("key", "csr", "crt"))
    openssl("req", "-new", "-newkey", "rsa:2048", "-nodes", "-keyout", key, "-out", csr,
            "-subj", f"/CN={name.title()}/emailAddress={email}")
    extensions = WORK / f"{name}.ext"
    extensions.write_text(
        "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\n"
        f"extendedKeyUsage=emailProtection\nsubjectAltName=email:{email}\n"
    )
    openssl("x509", "-req", "-in", csr, "-CA", str(WORK / f"{issuer}.crt"), "-CAkey",
            str(WORK / f"{issuer}.key"), "-days", "36500", "-out", cert, "-extfile", str(extensions))
    return key, cert


def authority(name, subject):
    key, cert = (str(WORK / f"{name}.{kind}") for kind in ("key", "crt"))
    openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", key, "-out", cert,
            "-days", "36500", "-subj", f"/CN={subject}",
            "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign")


def smime_message(sender, subject, signed):
    head, body = signed.replace(b"\r\n", b"\n").split(b"\n\n", 1)
    mime = [line for line in head.decode().split("\n") if not line.startswith("MIME-Version")]
    content_type = "\n".join(mime)
    return crlf(headers(sender, subject, content_type) + body.decode())


def smime():
    authority("ca", "Bande Test CA")
    authority("other-ca", "Unknown Test CA")
    alice = certificate("alice", "ca", address("alice"))
    carol = certificate("carol", "ca", address("carol"))
    mallory = certificate("mallory", "other-ca", "mallory@bande.test")
    save("ca.der", openssl("x509", "-in", str(WORK / "ca.crt"), "-outform", "DER"))
    openssl("pkcs12", "-export", "-inkey", alice[0], "-in", alice[1], "-passout", "pass:",
            "-out", str(HERE / "alice.p12"))
    part = crlf("Content-Type: text/plain; charset=utf-8\n\nCarol signed this.\n")
    signed = openssl("smime", "-sign", "-signer", carol[1], "-inkey", carol[0], "-md", "sha256", data=part)
    message = smime_message("carol", "S/MIME signed", signed)
    save("smime-verified.eml", message)
    save("smime-tampered.eml", message.replace(b"Carol signed this.", b"Carol signed that."))
    signed = openssl("smime", "-sign", "-signer", mallory[1], "-inkey", mallory[0], data=crlf(
        "Content-Type: text/plain\n\nMallory signed this.\n"))
    save("smime-unknown-issuer.eml", smime_message("mallory", "S/MIME unknown issuer", signed))
    opaque = openssl("smime", "-sign", "-nodetach", "-signer", carol[1], "-inkey", carol[0], data=part)
    save("smime-opaque.eml", smime_message("carol", "S/MIME opaque", opaque))
    inner = openssl("smime", "-sign", "-signer", carol[1], "-inkey", carol[0], data=crlf(
        "Content-Type: text/plain; charset=utf-8\n\nOnly Alice can read this.\n"))
    inner = b"\r\n".join(line for line in inner.replace(b"\r\n", b"\n").split(b"\n")
                         if not line.startswith(b"MIME-Version")).replace(b"\n", b"\r\n")
    encrypted = openssl("smime", "-encrypt", "-aes256", alice[1], carol[1], data=inner)
    save("smime-encrypted.eml", smime_message("carol", "S/MIME encrypted", encrypted))
    to_carol = openssl("smime", "-encrypt", "-aes256", carol[1], data=inner)
    save("smime-wrong-key.eml", smime_message("carol", "S/MIME for Carol", to_carol))
    gpgsm_home = home("gpgsm")
    openssl("pkcs12", "-export", "-inkey", carol[0], "-in", carol[1], "-certfile", str(WORK / "ca.crt"),
            "-passout", "pass:", "-out", str(WORK / "carol.p12"))
    gpgsm(gpgsm_home, "--import", str(WORK / "carol.p12"))
    (pathlib.Path(gpgsm_home) / "trustlist.txt").write_text(
        gpgsm(gpgsm_home, "--with-colons", "--list-keys", "Bande Test CA").decode().split("fpr:::::::::")[1].split(":")[0] + " S relax\n")
    detached = gpgsm(gpgsm_home, "--disable-crl-checks", "--armor", "--detach-sign", "--local-user",
                   address("carol"), data=part)
    body = (
        b"--sig\r\n" + part + b"\r\n--sig\r\n"
        + crlf("Content-Type: application/pkcs7-signature; name=smime.p7s\nContent-Transfer-Encoding: base64\n\n")
        + b"\r\n".join(line for line in detached.split(b"\n") if line and not line.startswith(b"-----"))
        + b"\r\n--sig--\r\n"
    )
    content_type = 'multipart/signed; boundary="sig"; micalg=sha-256; protocol="application/pkcs7-signature"'
    save("gpgsm-verified.eml", crlf(headers("carol", "gpgsm signed", content_type)) + body)


openpgp()
smime()
for folder in WORK.iterdir():
    if folder.is_dir():
        subprocess.run(["gpgconf", "--homedir", str(folder), "--kill", "all"], capture_output=True)
shutil.rmtree(WORK)
