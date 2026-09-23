# <div align="center"> CHANGELOG</div>

All notable changes to this project will be documented in this file.

This project adheres to [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [3.1.1] - 2026-09-23

### Fixed

- `dev` hot reload no longer loses rebuilds requested mid-build. The watcher loop polls with a timeout, so a change that lands while a build runs rebuilds right after.
- `dev` watches `src/`, `migrations/`, `seeds/`, `templates/`, `tests` plus manifest files instead of the whole project root. Watching the root pulled `target/` and `.git/` into the recursive watcher, so every build fired its own rebuild storm.
- `dev` resolves the binary name from the `[package]` section only. The old parse took the first `name =` line anywhere in the file, which picked the wrong binary in workspace roots.
- `dev` waits 5s for graceful shutdown before killing the old server. The old 2s deadline killed healthy servers mid-drain.
- The four `*_project_compiles` tests run under a mutex. They mutate the process working directory, so parallel threads stole each others cwd and failed with `NotFound`.

### Changed

- Removed 9 unused dependencies (template, cache, realtime, mail, storage, macros, utils, openapi, graphql, plugin). The CLI never imports them. This drops aws-sdk-s3 and juniper from every CLI build.
- Removed the `oxi` alias suggestion and all `oxi` references from docs. The command is `toxi`.

---

## [2.1.0] - 2026-25-01

###  Features

- N/A

###  Bug Fixes

- Fixed  and aliased ToxiRequest and ToxiResponse types to Request and Response respectively

###  Performance Improvements

- Improved Hot reload functioning.

###  Documentation

- N/A

###  Styling

- Enhanced the full-stack template with improved styling, a better HTML structure, and the Toxi logo.

###  Tests

- N/A

###  Miscellaneous

- N/A
