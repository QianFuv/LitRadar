# Swagger UI provenance

The 18 distribution files come from `utoipa-swagger-ui 9.0.2` embedded assets,
based on Swagger UI 5.17.14.
The original `v5.17.14.zip` SHA-256 is
`481244d0812097b11fbaeef79f71d942b171617f9c9f9514e63acbe13e71ccdc`.
Only `swagger-initializer.js` differs from the archive: the original crate's
build step installed its configuration placeholder. The Go handler fills that
placeholder with the docs router's four runtime settings.

LICENSE and NOTICE are copied from the archive root. No CDN is required.
The provenance and license files are not additional public docs endpoints.
