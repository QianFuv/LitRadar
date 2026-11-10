# Generated storage queries

Install the generator outside the application module:

```sh
go install github.com/sqlc-dev/sqlc/cmd/sqlc@v1.31.1
```

Run generation from `apps/server`:

```sh
go run ./cmd/sqlc-schema
sqlc generate
```

`sqlc-schema` runs the production auth, content, provider-control, index-batch,
and telemetry migrations against temporary databases. It exports their relational
tables into `schema/`; virtual FTS tables and SQLite shadow tables are excluded.
The migration implementations remain the schema authority. Commit both the SQL
inputs and generated Go packages. Do not edit either generated artifact by hand.

Check both inputs and outputs without changing them:

```sh
go run ./cmd/sqlc-schema -check
sqlc diff
```

Queries use `database/sql` with the existing modernc SQLite connections. Generated
methods accept the caller's connection or transaction, preserving immediate write
transactions, connection-local pragmas, audit rollback, and snapshot ownership.
The content writer uses sqlc's prepared query interface and retains ownership of
partially prepared statements so preparation failures also release resources.
Other existing batch loops retain their prepared statements and bind generated
SQL constants; their query definitions are maintained in `queries/`.

Type overrides preserve strict SQLite storage-class and UTF-8 validation,
nullable timestamps, and the distinction between NULL and empty BLOB values.
Notification preference reads retain raw SQLite values until the existing ordered
business validation runs. This preserves the precedence of malformed lists and
credential errors over later malformed preference columns.

FTS, runtime query composition, temporary membership tables, schema inspection,
and migration SQL remain handwritten. The atomic identity claim in
`index/content_sql.go` also remains handwritten: sqlc 1.31.1's SQLite parser rejects
`AS MATERIALIZED`, which preserves the owner observed before the upsert. Removing
that keyword would change the identity-claim semantics.
