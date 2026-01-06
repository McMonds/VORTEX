# The 12 Commandments of VORTEX Systems Programming

These rules are non-negotiable. Any violation of these commandments in the request path or core engine logic will result in immediate rejection.

## I. The Law of Panic Discipline
- **Rule**: `panic!`, `.unwrap()`, and `.expect()` are **Forbidden** in the Request Path.
- **Enforcement**: 
    - Use `unwrap()` ONLY during System Startup or in Tests.
    - Otherwise, propagate `Result<T, E>`. Keep the heartbeat alive.

## II. The Law of "Unsafe" Containment
- **Rule**: Every `unsafe` block must be accompanied by a `// SAFETY:` comment.
- **Enforcement**: Encapsulate raw pointer logic in tiny, localized modules. Expose safe APIs.

## III. The Law of Bounded Resources
- **Rule**: No collection (`Vec`, `HashMap`, `Queue`) is allowed to grow unbounded.
- **Enforcement**: Use `with_capacity()`. Check `len() >= MAX_LIMIT` before insertion.

## IV. The Law of Structured Observability
- **Rule**: No `println!` or raw text logs in hot paths.
- **Enforcement**: Use `tracing` with structured key-values.

## V. The Law of Zero-Magic
- **Rule**: No "Magic Numbers" in the code.
- **Enforcement**: Define constants in `config.rs`. Use descriptive names. Load from Env Vars.

## VI. The Law of Linear Control Flow
- **Rule**: Avoid "clever" logic. Prefer Early Returns (Guard Clauses).
- **Enforcement**: Keep happy-path logic on the left indentation margin.

## VII. The Law of Atomic Correctness
- **Rule**: Tests must be deterministic. 
- **Enforcement**: Use `TimeProvider` traits. No reliance on race conditions.

## VIII. The Law of FFI Hygiene
- **Rule**: Data crossing boundaries (Disk, Network) must be validated.
- **Enforcement**: "Parse, don't validate." Use `rkyv::check_archived_root`.

## IX. The Law of Public Documentation
- **Rule**: Every `pub` function must have a `/// docstring`.
- **Enforcement**: Document `Panics:` and `Errors:` conditions.

## X. The Law of Deadlock Avoidance
- **Rule**: Acquire resources in a fixed, global order.
- **Enforcement**: Adhere to Shard-per-Core (Lock-Free) whenever possible.

## XI. The Law of Branch Prediction
- **Rule**: Optimization focuses on cache lines and branches.
- **Enforcement**: Use `#[inline]` for hot small functions; `#[cold]` for error paths.

## XII. The Law of Defense in Depth
- **Rule**: Assume User/Hardware is trying to kill you.
- **Enforcement**: Recover from partial failures (e.g., Disk Full -> Read-Only). Rate limit internally.
