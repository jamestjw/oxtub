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

## Duplicate Index Entries

The current implementation stores non-unique index entries as fixed-width
physical entries ordered by `(K, Rid)`:

```text
(5, rid1)
(5, rid2)
(5, rid3)
```

This keeps duplicate handling deterministic. Leaf pages are sorted by `(K, Rid)`,
and internal separator keys also include `Rid` so insertion can preserve the same
total order.

Logical lookups by `K` do not need a specific `Rid`. They route to the leftmost
child that could contain `K`, then scan leaf entries to the right while the
logical key matches.

This layout is simple and correct, but duplicate-heavy indexes repeat the same
key many times.

## Tombstone-Aware Delete Rebalancing

Leaf-page tombstones are logical delete markers over physical entries ordered by
`(K, Rid)`. A tombstone stores an index into the leaf entry array; it does not
create a reusable free slot. Tombstoned entries still count toward
`curr_size()`, still consume leaf capacity, and still belong to the leaf's key
range because reinserting the same exact `(K, Rid)` can clear the tombstone.

Structural delete rebalancing is allowed to physically prune tombstoned entries.
It must never prune live entries. For now, favor code simplicity and correctness
over preserving tombstones for possible future reinsertion. Rebalancing should
therefore compact aggressively, keeping tombstones only when required to satisfy
the minimum physical occupancy invariant.

Important invariants:

- Leaf entries are physically sorted by `(K, Rid)`.
- `curr_size()` includes tombstoned entries.
- `num_tombstones <= TOMB_CAP`.
- Every tombstone index is unique and points to an existing physical entry.
- Non-root leaves satisfy `curr_size() >= min_size()`.
- Internal page slot `0` has only a child pointer.
- Internal page slot `i > 0` stores the lower-bound `(K, Rid)` for child `i`.
- Internal separators may be stale lower bounds after deletion.
- Plain deletion does not require separator repair.
- Redistribution requires separator repair because entries cross sibling ranges.

Exact deletes must route to the target leaf by full `(K, Rid)`, not by logical
`K` alone. Logical `K` routing is for lookup scans that need the leftmost
duplicate range. Exact delete should use the same full-index-key routing as
insertion.

### Leaf Pair Rebalancing

When a physical delete makes a non-root leaf underweight, rebalance that leaf
with one adjacent sibling. Prefer the left sibling when available; otherwise use
the right sibling.

The robust primitive is an adjacent-pair rebalance:

```text
rebalance_leaf_pair(left, right) -> Redistributed | Merged
```

The operation should materialize the two leaves as a sorted sequence of entries:

```text
(key, rid, is_tombstoned)
```

Then count:

```text
M = leaf.max_size()
m = leaf.min_size()
C = TOMB_CAP
P = total physical entries across the pair
T = total tombstoned entries across the pair
L = P - T
```

If `L > M`, the live entries cannot fit in one leaf, so the pair must remain two
leaves. Rebuild two leaves from live entries only. Tombstone capacity cannot
block this fallback because both rebuilt leaves have zero tombstones.

A valid split always exists. After a single underflowing physical delete:

```text
deficient_size = m - 1
sibling_size <= M
L <= P <= M + m - 1
L > M >= 2m
```

Choose `m` live entries for the left page and `L - m` live entries for the right
page:

```text
left_size = m
right_size = L - m
```

The left page is valid because it has exactly `m` entries.

The right page is not underweight because `L > M >= 2m`:

```text
L > M >= 2m
L >= 2m + 1
right_size = L - m
right_size >= m + 1
```

The right page is not overweight because `L <= P <= M + m - 1`:

```text
L <= M + m - 1
right_size = L - m
right_size <= M - 1
```

Therefore both pages satisfy occupancy. Since the fallback redistribution uses
live entries only, both pages have zero tombstones and automatically satisfy
`num_tombstones <= TOMB_CAP`. Sorted order is preserved by taking the first `m`
live entries for the left page and the remaining live entries for the right page.

If `L <= M`, the live entries fit in one leaf, so the pair can merge. If
`L >= m`, merge with live entries only. If `L < m`, retain exactly `m - L`
tombstoned entries as physical filler and prune all other tombstones. This is the
only case where structural rebalance should keep tombstones for now.

The filler amount is always available and fits tombstone capacity:

```text
q = m - L
q <= T
q <= C
```

`q <= T` because the pair has enough total physical entries before compaction:

```text
P = L + T
P >= 2m - 1 >= m
T = P - L >= m - L = q
```

`q <= C` is immediate when `C >= m`. When `C < m`, each source page has at most
`C` tombstones, so:

```text
L >= (m - 1 - C) + (m - C)
L >= 2m - 1 - 2C
q = m - L
q <= 2C - m + 1
q <= C
```

The last step follows from `C < m`. Therefore, merge cannot fail due tombstone
capacity.

This makes leaf rebalancing total as long as structural repair may compact
tombstones. A raw one-entry borrow or raw physical merge may still be used as an
optimization later, but it should not be the correctness primitive.

### Correctness-First Rebalance Policy

We use this order for deletes:

```text
1. If the delete only adds a tombstone, stop.
2. If physical delete does not underflow, stop.
3. If a non-root leaf underflows, run adjacent-pair rebalance.
4. If rebalance redistributes two leaves, update the parent separator for the
   right leaf.
5. If rebalance merges into one leaf, remove the deleted child pointer from the
   parent and propagate parent underflow if needed.
```

For simplicity, adjacent-pair rebalance should compact tombstones aggressively.
Preserving extra tombstones or minimizing entry movement is a future
optimization; correctness only requires keeping live entries and enough
tombstoned filler to satisfy non-root minimum occupancy after a merge.

A useful future optimization is to prefer two-page redistribution over merge
whenever both are valid. Redistribution only rewrites the two sibling leaves and
updates one parent separator, so repair stops at the leaf level. Merge removes a
child pointer from the parent, deletes a page, and may make the parent
underweight, causing underflow repair to propagate up the tree. The
correctness-first implementation may merge whenever `L <= M`; an optimized
implementation can first check whether retaining enough tombstoned filler allows
both pages to remain valid, and redistribute instead when possible.

## Future Duplicate Deduplication With Posting Lists

A future optimization can store duplicate keys as posting-list cells:

```text
5 -> [rid1, rid2, rid3]
6 -> [rid4]
```

For fixed-width `K`, a posting-list leaf page should likely use a slotted-page
layout:

```text
header
slot directory, sorted by K
free space
variable-size posting-list cells growing from the end
```

Each cell stores:

```text
K
rid_count
Rid[rid_count]
```

Posting-list cells should have a maximum physical size or maximum RID count. The
point of this limit is not that two adjacent lists are logically better than one
long list. The point is that every cell must remain small enough to fit on a page
and be moved, split, and compacted normally.

When a posting list reaches the limit, create another adjacent cell with the same
logical key:

```text
5 -> [rid1..rid64]
5 -> [rid65..rid128]
```

These cells are logically one duplicate group, but physically they are bounded
chunks. Lookup for all duplicates scans adjacent cells while `K` matches.

To preserve internal-page fanout, future posting-list internal pages may use
`K`-only separators instead of `(K, Rid)` separators. This keeps internal keys
smaller, at the cost of less precise insertion routing for duplicate-heavy keys.

For insertion into a duplicate-heavy key, route to the leftmost leaf that could
contain `K`, then scan right through leaf siblings until the target `(K, Rid)`
belongs. The required page-range metadata does not need to be stored in the leaf
header. It can be computed from the first and last posting-list cells:

```text
first_index_key = (first_cell.K, first_cell.first_rid)
last_index_key  = (last_cell.K,  last_cell.last_rid)
```

This avoids duplicated metadata that can go stale. If duplicate insertion scans
become too expensive, we can reconsider `(K, first_rid)` internal separators or a
duplicate-specific structure.

If a page has no room for a new or expanded posting-list cell, split the leaf by
bytes used, not by number of cells. Overflow pages are more general, but should
be avoided initially because they add complexity around deletion, scans, page
lifecycle, and compaction.

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
