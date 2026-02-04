# DEVNOTES

This file contains developer-focused instructions: signing, verification, and CI tips.

## Signing & verifying binaries (recommended)

To let others verify downloaded or built binaries, publish a checksum and a detached signature. A simple, cross-platform approach uses GPG detached signatures and SHA256 checksums.

- Create a SHA256 checksum:

```bash
sha256sum target/release/duptool > duptool.sha256
```

- Create a detached GPG signature (recommended for cross-platform verification):

```bash
gpg --output duptool.sig --detach-sign target/release/duptool
gpg --armor --output duptool.sig.asc --detach-sign target/release/duptool
```

- Publish `duptool`, `duptool.sha256`, and `duptool.sig.asc` (or `duptool.sig`) in your release assets. Also publish your ASCII-armored public key (`duptool_public.gpg`) so users can import it.

- Verify locally (user side):

```bash
sha256sum -c duptool.sha256            # quick checksum verification
gpg --import duptool_public.gpg       # one-time (or trust the key via web-of-trust)
gpg --verify duptool.sig.asc duptool  # verify signature
```

## Windows Authenticode (optional)

If you publish Windows executables and want Authenticode signatures, use `osslsigncode` (open-source) or Microsoft's `signtool` in a CI runner that has access to a code-signing certificate. For reproducible verification, also publish a checksum and the GPG signature as above so non-Windows users can still validate files.

Example (osslsigncode):

```bash
osslsigncode sign -certs cert.pem -key key.pem -n "duptool" -in duptool.exe -out duptool-signed.exe
```

CI tip: keep the private signing key in a secure secret store (GitHub Actions secrets, Azure Key Vault, etc.) and perform signing as a release step.
