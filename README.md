# datakeel

**Blazing-fast, agent-first (DX + AX) Rust data contract, schema drift, and codegen CLI with native [Open Data Contract Standard (ODCS v3)](https://bitol-io.github.io/open-data-contract-standard/) support.**

`datakeel` replaces slow, dependency-heavy Python data quality tools with a single compiled Rust binary designed for **both human engineers (DX) and AI coding agents (AX)**.

## Why `datakeel`?

- **Open-standard + ergonomic:** Reads and writes **ODCS v3.2.0 YAML** (`datacontract.yaml`) natively *and* compact **`datakeel.toml`** files through one unified Rust model.
- **Zero-bloat multi-format engine:** Profiles and validates **JSON**, **JSONL / NDJSON**, **CSV / TSV**, and **Parquet** natively (plus `stdin`).
- **Cross-source referential integrity:** Catch orphan IDs across API extracts (e.g. Canvas `sis_user_id` → Populi `student_id`) *before* a SQL `INNER JOIN` silently drops records.
- **Schema & distribution drift detection:** Type-widening compatibility lattice, null-rate spike alerts, and row-count drop guards (catches truncated API pagination!).
- **Single source of truth codegen:** Generate **PostgreSQL `CREATE TABLE` DDL** and **Python Pydantic v2 models** directly from your contract so humans and agents never drift on column names or types.
- **First-class AX (Agent Experience):**
  - **TTY-aware dual mode:** Colorized human output in a terminal; compact single-line JSON (`--format agent`) automatically when piped.
  - **Strict `stdout` / `stderr` separation:** `stdout` is pure data/code; `stderr` holds structured JSON error envelopes.
  - **Token-budgeted diagnostics:** Violations are deduplicated by rule + column, capped at 3 sample rows (`--max-samples`), and paired with stable `DK_*` codes and `fix_hint`s.
  - **Bundled `SKILL.md`:** Run `datakeel skill --install .agents/skills/datakeel` so any coding agent immediately knows every command, exit code, and workflow.

## Quickstart

```bash
# 1. Infer an ODCS v3.2.0 contract from a raw JSONL/CSV/Parquet dump
datakeel infer examples/seminary-etl/populi_students.jsonl \
  --infer-enums 10 \
  -o populi_students.datacontract.yaml

# 2. Validate with cross-source foreign-key integrity
datakeel check \
  examples/seminary-etl/canvas_enrollments.toml \
  examples/seminary-etl/canvas_enrollments_bad.jsonl \
  --ref populi_students=examples/seminary-etl/populi_students.jsonl

# 3. Catch API schema drift, null spikes, and pagination drops between runs
datakeel diff \
  examples/seminary-etl/populi_students.jsonl \
  examples/seminary-etl/populi_students_today_drift.jsonl

# 4. Generate Postgres DDL & Python Pydantic v2 models from one contract
datakeel codegen examples/seminary-etl/populi_students.datacontract.yaml --target postgres --schema staging
datakeel codegen examples/seminary-etl/populi_students.datacontract.yaml --target pydantic

# 5. Install the built-in AI Agent Skill into your repository
datakeel skill --install .agents/skills/datakeel
```

## Exit Codes

| Code | Meaning |
| :--- | :--- |
| `0` | Clean pass / no breaking drift / valid contract |
| `1` | Contract violation, breaking drift, or contract lint failure |
| `2` | CLI usage, IO, or parse error (`ErrorEnvelope` on `stderr` in `agent`/`json` mode) |

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).
