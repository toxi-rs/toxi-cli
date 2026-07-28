# toxi-cli

Command-line tooling for Toxi. The package name is `toxi-cli`, and the installed binary is `toxi`.

## Installation

```bash
# Install from crates.io
cargo install toxi-cli

# Install this generated CLI build explicitly
cargo install toxi-cli --version 3.1.0

# Install from the local checkout
cargo install --path .
```

Verify the binary:

```bash
toxi --version
toxi version
```

## Project Creation

```bash
toxi new my-app
toxi new my-api --project-type api
toxi new my-api --type api
toxi new my-web --template web
toxi new my-minimal --template minimal
```

Generated projects include:

```text
my-app/
├── Cargo.toml
├── README.md
├── toxi.toml
├── migrations/
├── seeds/
├── src/
│   ├── main.rs
│   ├── controllers/
│   ├── events/
│   ├── jobs/
│   ├── middleware/
│   ├── models/
│   ├── policies/
│   ├── routes/
│   ├── services/
│   └── validators/
└── tests/
```

Supported project kinds:

- `api`
- `fullstack`
- `web` as an alias for `fullstack`
- `microservice`
- `minimal` as an alias for `api`
- `serverless`

## Generators

Use `generate` for new workflows. `make` remains as a hidden compatibility alias.

```bash
toxi generate model User
toxi generate model User email:string age:integer
toxi generate route users
toxi generate controller UserController
toxi generate middleware AuthMiddleware
toxi generate service Billing
toxi generate validator CreateUser
toxi generate job SendDigest
toxi generate policy Post
toxi generate event UserSignedUp
toxi generate migration create_users_table
toxi generate seeder users_seed
```

Supported model field types:

- `string`
- `text`
- `integer`
- `float`
- `decimal`
- `boolean`
- `uuid`
- `json`
- `timestamp`

## Migrations

Create a migration:

```bash
toxi migrate create create_users_table
toxi generate migration create_users_table
```

Migration files are SQL files with `-- migrate:up` and `-- migrate:down` sections:

```sql
-- migrate:up
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL
);

-- migrate:down
DROP TABLE users;
```

Run or inspect migrations:

```bash
toxi migrate
toxi migrate run
toxi migrate status
toxi migrate revert
toxi migrate:rollback
```

## Seeders

```bash
toxi seed create users_seed
toxi generate seeder users_seed
toxi seed
toxi seed run
toxi db:seed
```

## Queue Commands

Canonical commands:

```bash
toxi queue work --workers 4
toxi queue list
toxi queue dlq
toxi queue clear
```

Compatibility aliases:

```bash
toxi queue:work --workers 4
toxi queue:list
toxi queue:dlq
toxi queue:clear
```

## Development Workflow

```bash
toxi dev
toxi dev --port 8080
toxi dev --host 0.0.0.0 --env development
toxi dev --watch src --watch templates
toxi dev --ignore dist
toxi dev --no-hot-reload
```

`toxi dev` forwards the selected host, port, and environment through:

- `SERVER_HOST`
- `SERVER_PORT`
- `TOXI_ENV`

Build and run the current project:

```bash
toxi build
toxi build --release
toxi build --profile release
toxi build --target x86_64-unknown-linux-musl
toxi build --features "database,queue"
toxi build --verbose

toxi serve
toxi serve --addr 0.0.0.0:8080
toxi serve --env production
```

## Diagnostics

```bash
toxi doctor
toxi --help
toxi migrate --help
toxi generate --help
```

The generated project configuration file is `toxi.toml`.

## License

MIT OR Apache-2.0
