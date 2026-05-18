# oxtub

`oxtub` is an experimental Rust rewrite of [BusTub](https://github.com/cmu-db/bustub), the educational relational database system from Carnegie Mellon University's Database Group.

The goal of this project is to explore database internals in Rust while following the broad architecture and learning goals of BusTub. It is intended as a learning project, not a production database.

## Status

This project is in a very early stage. Core components such as disk management, page management, buffering, storage, execution, and query processing are expected to evolve over time.

## Goals

- Reimplement BusTub-inspired database components in idiomatic Rust.
- Use Rust's type system and error handling to make storage and execution code explicit and safe.
- Keep the codebase approachable for learning database systems internals.

## Attribution

BusTub is developed by the CMU Database Group. This project is independently maintained and is not affiliated with or endorsed by CMU.
