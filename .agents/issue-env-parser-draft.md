## Summary

Komodo currently uses a custom environment parser for stack and repo environment values in `client/core/rs/src/parsers.rs`. The parser accommodates plain `KEY=value` lines, a pseudo-YAML list form (`- KEY:value`), and TOML-embedded strings. However, as the project grows, handling these mixed formats in a single custom parser introduces some edge cases (such as with multiline values and quote handling).

To make the environment parsing more robust and easier to maintain, I would like to propose migrating this to a mature, dedicated parser (like `dotenvy`, which is already a dependency) rather than extending the current custom implementation. I'm very open to feedback on whether this aligns with the project's goals!

## Why this matters

The current parser behaves like a hybrid of a partial dotenv parser and a partial YAML-like parser. This introduces a few practical challenges:

1. **Quotes and inline comments:** The parser removes anything after the first `" #"` substring. According to standard Docker Compose `.env` syntax, `VAR="VAL # not a comment"` should preserve the hash. Since the current parser doesn't track whether it's inside quotes, `KEY="abc # def"` gets truncated to `KEY="abc`. Additionally, preserving outer quotes can conflict with standard `.env` behaviors where wrapping quotes are stripped during parsing.
2. **Escape/Interpolation support:** Docker Compose `.env` files support variable interpolation and escape sequences (`\n`, `\t`, etc.). The current implementation relies on a `trim()` and `split_once` approach, which makes supporting these advanced semantics tricky. Since Komodo already depends on `dotenvy`, leveraging it here could bring native support for multiline values and variable substitution out of the box.
3. **Quote leakage downstream:** Because the custom parser preserves outer quotes, `INTERVAL="1-day"` becomes `"1-day"` instead of `1-day`. When injected into shells or other config paths, these literal quotes can sometimes cause unexpected side effects (somewhat related to the patterns seen in #390).
4. **Pseudo-YAML parsing:** Handling lines like `- KEY: value` via manual line splitting skips proper YAML rules (for quoted scalars, block strings, etc.). If Komodo intends to support structured lists within YAML/TOML configs, it might be safer to let `serde_yaml_ng` or `toml` deserialize directly into `Vec<EnvironmentVar>` instead of guessing via a line-oriented parser.
5. **User-facing error messages:** Multiline values currently surface as `line n missing assignment character ('=' or ':')` because the parser treats the continuation line as a new variable (as seen in #738).

## Proposed direction

I would love to suggest splitting the responsibilities:

* Use a mature dotenv parser for plain environment text inputs.
* Use proper YAML/TOML deserializers if structured list configurations need to be supported.

Personally, I feel dropping the pseudo-YAML list compatibility might simplify both the implementation and the user experience. However, if the maintainers prefer to keep that format, we could route it through a proper YAML parsing layer instead of the current custom logic.

## Expected outcome

This change would give Komodo a cleaner separation of concerns:

* Standard `.env` text input handled by a robust dotenv parser.
* Structured config input handled by appropriate format parsers.

This should naturally resolve current edge cases around quotes and multiline variables, making the behavior predictable for users.

## Offer

If the maintainers are open to this direction, I would be more than happy to draft a Pull Request for it!

Before diving in, I wanted to check: **Is the current YAML-like list support (`- KEY:value`) still considered a required feature for the community?** If we can deprecate it, the cleanup will be very straightforward. If it's still needed, I can make sure to implement a robust YAML parsing path to preserve it. Thank you for your time and for maintaining such a great project!
