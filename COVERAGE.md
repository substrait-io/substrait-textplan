<!-- SPDX-License-Identifier: Apache-2.0 -->
# Substrait proto feature coverage

Tracks which Substrait proto features the textplan converter supports in **both**
directions (binary→text printer and text→binary parser). Gaps show up in the
printer as `*_NOT_YET_IMPLEMENTED` markers (the printer matches are exhaustive,
so these are a complete inventory of missing features at spec 0.85).

Legend: ✅ done · ⬜ stubbed

| Feature | Kind | Group PR | Status |
|---|---|---|---|
| `Struct` / `List` / `Map` | type | C1 compound-types | ✅ |
| `Map` | literal | C2 map-literals | ✅ |
| `Struct` | literal | C2b struct-literals | ✅ |
| `List` / `EmptyList` / `EmptyMap` | literal | C2c list/empty-literals (needs grammar) | ⬜ |
| `Binary` / `Timestamp` / `TimestampTz` / `UUID` | literal | C3 scalar-literals | ⬜ |
| `PrecisionTime` / `PrecisionTimestamp` / `PrecisionTimestampTz` / `IntervalCompound` | type + literal | C4 precision-temporal | ⬜ |
| `MaskedReference` / `ListElementReference` / `MapKeyReference` | field reference | C5 field-references | ⬜ |
| `SwitchExpression` / `SingularOrList` / `MultiOrList` | expression | C6 conditional-exprs | ⬜ |
| `Nested` (struct/list/map builders) | expression | C7 nested-constructors | ⬜ |
| `WindowFunction` | expression | C8 window-functions | ⬜ |
| `UserDefined` type / ref / literal | type + literal | C9 user-defined | ⬜ |
| `Lambda` / `LambdaInvocation` / `Func` type | expression + type | C10 lambda (spec #889) | ⬜ |
| `TypeAliasReference` / `type_alias_reference` | type | C11 type-aliases (spec #868) | ⬜ |
| `DynamicParameter` (+ plan `parameter_bindings`) | expression | C12 dynamic-parameters | ⬜ |

## After coverage is clean

- **Cutover to substrait-packaging** (`substrait-prost`, spec 0.85→0.87) — mechanical.
- **Execution context / behavior** (`ExecutionContextVariable`, `ExecutionBehavior`) — spec 0.87 (#945).

## Notes

- Relations (all 22 `Rel.rel_type` variants) are already handled.
- Protos are cumulative, so auditing against the current spec (0.85) catches
  every feature; no need to replay releases.
