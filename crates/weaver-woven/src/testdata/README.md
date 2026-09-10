# Public shutdown-test TLS identity

`cert.pem.txt` and `key.pem.txt` are a throwaway, publicly known test identity,
used only by the secure-remote shutdown test on ephemeral `127.0.0.1` listeners.
They must never be deployed, used as real credentials, or trusted outside tests.
The `.txt` suffix keeps these deliberate fixtures distinct from ignored secret
PEM files. The test writes temporary owner-only files and removes them afterward.

The self-signed P-256/SHA-256 end-entity certificate has IP SAN `127.0.0.1`,
`CA=false`, and validity January 1, 2020 through January 1, 2036. Regenerate before
expiry with the same constraints and a new throwaway key. Tests use standard
certificate, validity, and IP verification; they never disable TLS validation.
No certificate-generation executable or extra dependency is needed to run tests.
