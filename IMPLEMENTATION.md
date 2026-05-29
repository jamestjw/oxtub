# Implementation Notes

## Fixed-Width Index Keys

For fixed-width integer-based indexes, use `GenericKey<N>`.

`GenericKey<N>` stores order-preserving encoded bytes:

```rust pub struct GenericKey<const N: usize> { data: [u8; N], } ```

The B-tree can compare these keys bytewise, as long as each concrete SQL/Rust
type is encoded so byte order matches logical sort order.

Examples:

- `bigint` / `i64` maps to `GenericKey<8>`
- `integer` / `i32` maps to `GenericKey<4>`
- `(i32, i32)` maps to `GenericKey<8>`
- `(i64, i32)` maps to `GenericKey<12>`

Signed integers must use sortable encoding, not raw little-endian bytes.

For `i64`:

```rust (value ^ i64::MIN).to_be_bytes() ```

For `i32`:

```rust (value ^ i32::MIN).to_be_bytes() ```

The XOR flips the sign bit so signed ordering becomes unsigned byte ordering.
Big-endian bytes then preserve numeric order under lexicographic comparison.

Composite keys are encoded by concatenating each column's sortable encoding in
index-column order.

For an index on `(a: i32, b: i32)`:

```text encode_i32(a) || encode_i32(b) ```

This preserves tuple ordering:

```text (a1, b1) < (a2, b2) if a1 < a2, or a1 == a2 and b1 < b2 ```

## Composite Range Scans

For an index on `(a, b)`, a query like:

```sql WHERE a = x AND b >= y ```

can use a range scan.

Lower bound:

```text encode_i32(x) || encode_i32(y) ```

Then scan forward while the key prefix still matches:

```text key[0..4] == encode_i32(x) ```

This avoids decoding keys during the scan. Comparisons remain bytewise.

This works well because equality is on the leading index column and the range
predicate is on the next column.

A query like:

```sql WHERE b >= y ```

cannot efficiently use an `(a, b)` index as a direct range scan, because `b` is
not the leading key column.

## String Indexes

String keys are variable-length and may depend on collation, so they should not
initially be forced into `GenericKey<N>` as exact keys.

Start with a prefix-key design:

```rust pub struct StringPrefixKey<const N: usize> { data: [u8; N], } ```

For now, we will use binary collation:

```text compare UTF-8 bytes lexicographically ```

Encoding:

- take the first `N` bytes of the string
- zero-pad the remaining bytes
- store the prefix in the B-tree
- use the base table row as the source of truth for full-string verification

String prefix indexes are useful for navigation but are not exact for all cases.
Prefix collisions are possible, so lookups must recheck the full string against
the table row.

For example:

```sql WHERE name = 'alice' ```

Index lookup can use `StringPrefixKey<N>` for `'alice'`, but matching rows must
be verified by reading the full `name` value from the table.

For uniqueness checks, prefix keys alone are insufficient unless the full string
is also stored or verified against all prefix matches.

## Future String Index Design

A more complete string index should use variable-length index entries.

Possible layout:

```text slot directory grows from the front key/value payload grows from the
back ```

Each slot contains:

```text key_offset key_len rid ```

Page binary search uses a comparator over the variable-length key bytes.

This supports exact string keys and better SQL semantics, but it makes page
layout, compaction, splitting, and comparison more complex.

## Runtime Index Metadata

The B-tree page layer is typed and layout-oriented. Runtime index metadata
should decide how to encode and compare keys.

Index metadata should include:

- indexed columns
- physical key encoding
- key width for fixed-width keys
- string prefix length for prefix indexes
- collation, eventually
- uniqueness, eventually

Lookup should dispatch once based on metadata, then run the appropriate typed
implementation.

Example:

```rust match index.key_kind { KeyKind::Generic8 =>
search::<GenericKey<8>>(...), KeyKind::Generic12 =>
search::<GenericKey<12>>(...), KeyKind::StringPrefix32 =>
search::<StringPrefixKey<32>>(...), } ```

Avoid decoding page bytes into dynamic values for every comparison. Instead:

- encode the lookup key once
- compare encoded bytes inside the B-tree
- recheck full values only when necessary, such as string prefix matches
