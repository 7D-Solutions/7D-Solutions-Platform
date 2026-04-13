#!/usr/bin/env python3
"""
Event catalog generator for the 7D Solutions Platform.

Scans:
  contracts/events/*.v1.json   — JSON Schema contracts (title, description, payload)
  modules/*/module.toml        — outbox publisher config (outbox_table, subject_prefix)
  modules/*/src/**/*.rs        — subject constants + consumer subscriptions

Outputs (deterministic):
  contracts/events/catalog.json  — machine-readable catalog
  docs/event-catalog.md          — human-readable markdown table

Usage:
  python3 tools/event-catalog/generate.py

CI validation:
  python3 tools/event-catalog/generate.py --check
  Exits 1 if the generated output differs from committed files.
"""

import argparse
import json
import os
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
CONTRACTS_DIR = ROOT / "contracts" / "events"
MODULES_DIR = ROOT / "modules"
CATALOG_JSON = CONTRACTS_DIR / "catalog.json"
CATALOG_MD = ROOT / "docs" / "event-catalog.md"


# ── helpers ──────────────────────────────────────────────────────────────────

def module_name_from_path(path: Path) -> str:
    """Derive module name from a path inside modules/<name>/..."""
    parts = path.relative_to(MODULES_DIR).parts
    return parts[0] if parts else "unknown"


# ── Step 1: Parse JSON schema contracts ──────────────────────────────────────

def parse_json_schemas():
    """
    Returns dict: event_type -> {description, publisher, payload_fields, schema_file}
    The 'title' field in each schema is the event_type (not the wire subject).
    """
    schemas = {}
    for f in sorted(CONTRACTS_DIR.glob("*.v1.json")):
        if f.name == "catalog.json":
            continue
        try:
            doc = json.loads(f.read_text())
            title = doc.get("title", "")
            if not title:
                continue
            props = doc.get("properties", {})
            sm = props.get("source_module", {})
            publisher = sm.get("const", "")
            description = doc.get("description", "")
            payload_obj = props.get("payload", {})
            payload_props = payload_obj.get("properties", {})
            payload_fields = list(payload_props.keys())
            schemas[title] = {
                "description": description,
                "publisher": publisher,
                "payload_fields": payload_fields,
                "schema_file": f"contracts/events/{f.name}",
            }
        except Exception as e:
            print(f"WARNING: could not parse {f}: {e}", file=sys.stderr)
    return schemas


# ── Step 2: Parse module.toml files ──────────────────────────────────────────

def parse_module_configs():
    """
    Returns list of dicts with module metadata.
    Each entry: {name, version, has_outbox, outbox_table, subject_prefix, subscribed_subjects}
    """
    try:
        import tomllib
    except ImportError:
        try:
            import tomli as tomllib  # type: ignore[no-redef]
        except ImportError:
            # Fallback: minimal TOML parser for the fields we need
            tomllib = None  # type: ignore[assignment]

    modules = []
    for toml_path in sorted(MODULES_DIR.glob("*/module.toml")):
        module_dir = toml_path.parent
        name = module_dir.name
        text = toml_path.read_text()

        outbox_table = ""
        subject_prefix = ""
        has_outbox = False
        subscribed_subjects: list[str] = []

        if tomllib is not None:
            try:
                data = tomllib.loads(text)
            except Exception:
                data = {}
            mod_section = data.get("module", {})
            name = mod_section.get("name", name)
            events = data.get("events", {})
            publish = events.get("publish", {})
            if publish:
                has_outbox = True
                outbox_table = publish.get("outbox_table", "")
                subject_prefix = publish.get("subject_prefix", "")
            subscribe = events.get("subscribe", {})
            if subscribe:
                subj = subscribe.get("subjects", [])
                if isinstance(subj, list):
                    subscribed_subjects = subj
        else:
            # Minimal regex-based TOML parsing
            m = re.search(r'\[module\].*?^name\s*=\s*"([^"]+)"', text, re.DOTALL | re.MULTILINE)
            if m:
                name = m.group(1)

            if "[events.publish]" in text:
                has_outbox = True
                ot = re.search(r'outbox_table\s*=\s*"([^"]+)"', text)
                if ot:
                    outbox_table = ot.group(1)
                sp = re.search(r'subject_prefix\s*=\s*"([^"]+)"', text)
                if sp:
                    subject_prefix = sp.group(1)

            if "[events.subscribe]" in text:
                subj_m = re.search(r'subjects\s*=\s*\[([^\]]+)\]', text)
                if subj_m:
                    subscribed_subjects = re.findall(r'"([^"]+)"', subj_m.group(1))

        modules.append({
            "name": name,
            "dir": module_dir.name,
            "has_outbox": has_outbox,
            "outbox_table": outbox_table,
            "subject_prefix": subject_prefix,
            "subscribed_subjects": subscribed_subjects,
        })

    return modules


# ── Step 3: Scan Rust source for subjects ────────────────────────────────────

# Regex patterns for finding NATS subjects in Rust source.
# We match only explicit SUBJECT_* / NATS_SUBJECT_* / status-subject constants.
# EVENT_TYPE_* constants are intentionally excluded — they are event_type values,
# not wire subjects. Wire subjects for those come from JSON schemas + prefix mapping.
SUBJECT_CONST_RE = re.compile(
    r'(?:pub\s+)?const\s+(?:SUBJECT_\w+|NATS_SUBJECT_\w+|WO_\w+|PLAN_\w+|ASSET_\w+|'
    r'METER_\w+|DOWNTIME_\w+|CALIBRATION_\w+|OUT_OF_SERVICE_\w+|'
    r'INSTANCE_\w+|DEFINITION_\w+|STEP_\w+|HOLD_\w+|ESCALATION_\w+|DELEGATION_\w+|'
    r'BILLING_RUN_\w+|PARTY_INVOICED)'
    r'\s*:\s*&str\s*=\s*"([^"]+)"',
    re.MULTILINE
)

# Literal subject assignments: let subject = "some.nats.subject";
LET_SUBJECT_RE = re.compile(
    r'let\s+subject\s*=\s*"([^"]+)"'
)

# format!("{}.events.{}", ...) publisher prefix pattern extraction
FORMAT_EVENTS_PREFIX_RE = re.compile(
    r'format!\s*\(\s*"([^"]+)\.events\.\{\}"'
)

# Subscriptions in module.toml [events.subscribe] already handled above

# Subjects that are clearly test-only — skip them
TEST_SUBJECTS = {
    "test.subject",
    "test.events.test",
    "smoke_test.item_created",  # keep this one — it's smoke-test module
}

def looks_like_nats_subject(s: str) -> bool:
    """A NATS subject has at least one dot and no spaces."""
    return "." in s and " " not in s and len(s) < 120


def scan_rust_files():
    """
    Scans all Rust source files under modules/.

    Returns:
      subjects_from_source: dict[subject_str -> {publisher_module, consumer_modules: set}]
      publisher_prefixes: dict[module_name -> prefix_str]  (e.g. "ar" -> "ar.events")
    """
    subjects: dict[str, dict] = defaultdict(lambda: {"publisher_module": "", "consumer_modules": set()})
    publisher_prefixes: dict[str, str] = {}

    for rs_file in sorted(MODULES_DIR.rglob("*.rs")):
        # Skip test files and build artifacts
        rel = rs_file.relative_to(ROOT)
        parts_str = str(rel)
        if "target/" in parts_str:
            continue

        mod_name = module_name_from_path(rs_file)
        text = rs_file.read_text(errors="replace")

        # Detect publisher prefix patterns:  format!("{prefix}.events.{}", ...)
        for m in FORMAT_EVENTS_PREFIX_RE.finditer(text):
            prefix = m.group(1)
            if prefix and "." not in prefix or (prefix and prefix.count(".") == 0):
                # Simple prefix like "ar", "inventory", "payments"
                publisher_prefixes[mod_name] = f"{prefix}.events"
            elif prefix:
                publisher_prefixes[mod_name] = prefix

        # Classify the file as consumer-side or publisher-side
        is_consumer = (
            "/consumers/" in parts_str
            or "/consumer_tasks" in parts_str
            or "/ingest/" in parts_str
        )
        is_publisher = (
            "/outbox" in parts_str
            or "/publisher" in parts_str
            or "/event_bus" in parts_str
            or "/events/subjects" in parts_str
        )

        # Detect const subject declarations
        for m in SUBJECT_CONST_RE.finditer(text):
            subject = m.group(1)
            if not looks_like_nats_subject(subject):
                continue
            entry = subjects[subject]
            if is_consumer:
                entry["consumer_modules"].add(mod_name)
            elif is_publisher:
                if not entry["publisher_module"]:
                    entry["publisher_module"] = mod_name
            else:
                # Shared or dispatch files — classify by subject prefix
                # If subject starts with module's own prefix, it's a publisher constant
                # If it starts with a different module's prefix, it's a consumer reference
                pass  # leave ambiguous; publisher resolved by prefix inference later

        # Detect let subject = "..." assignments
        for m in LET_SUBJECT_RE.finditer(text):
            subject = m.group(1)
            if not looks_like_nats_subject(subject):
                continue
            entry = subjects[subject]
            if is_consumer:
                entry["consumer_modules"].add(mod_name)
            elif is_publisher:
                if not entry["publisher_module"]:
                    entry["publisher_module"] = mod_name
            # else: ambiguous — don't classify

    return subjects, publisher_prefixes


# ── Step 4: Scan EVENT_TYPE constants from module events directories ───────────
#
# For modules with outbox configs but no JSON schema contracts, we derive wire
# subjects from EVENT_TYPE_* constants defined in the module's events/ directory.
# This covers treasury, workforce-competence, bom, and other schema-less publishers.

# Matches EVENT_TYPE_* and EVT_* module-local event type constants
MODULE_EVENT_CONST_RE = re.compile(
    r'(?:pub\s+)?const\s+(?:EVENT_TYPE_\w+|EVT_\w+)\s*:\s*&str\s*=\s*"([^"]+)"',
    re.MULTILINE
)

def scan_module_event_types(module_configs: list[dict], known_prefixes: dict[str, str]):
    """
    For outbox-enabled modules, scan ALL source files for EVENT_TYPE_* and EVT_*
    constants and derive wire subjects. Only includes subjects not already present
    in JSON schema contracts.

    Returns: list of (subject, publisher_module, event_type)
    """
    results: list[tuple[str, str, str]] = []
    seen: set[tuple[str, str]] = set()  # (module, wire_subject) dedup

    for mod in module_configs:
        mod_name = mod["name"]
        src_dir = MODULES_DIR / mod["dir"] / "src"
        if not src_dir.exists():
            continue
        # Prefer explicit subject_prefix, then known prefix map, then no prefix
        prefix = mod.get("subject_prefix") or known_prefixes.get(mod_name, "")

        for rs_file in sorted(src_dir.rglob("*.rs")):
            rel = str(rs_file)
            if "target/" in rel or "/tests/" in rel or "/bin/" in rel:
                continue
            text = rs_file.read_text(errors="replace")
            for m in MODULE_EVENT_CONST_RE.finditer(text):
                et = m.group(1)
                if not looks_like_nats_subject(et):
                    continue
                if prefix:
                    wire_subject = f"{prefix}.{et}"
                else:
                    wire_subject = et
                key = (mod_name, wire_subject)
                if key not in seen:
                    seen.add(key)
                    results.append((wire_subject, mod_name, et))
    return results


# ── Step 5: Scan for ALL_SUBJECTS arrays (maintenance, workflow) ──────────────

ALL_SUBJECTS_ARRAY_RE = re.compile(
    r'pub\s+const\s+ALL_SUBJECTS\s*:\s*&\[&str\]\s*=\s*&\[([^\]]+)\]',
    re.DOTALL
)

def scan_all_subjects_arrays():
    """Returns dict[module_name -> list[subject]]"""
    result: dict[str, list[str]] = {}
    for rs_file in sorted(MODULES_DIR.rglob("*.rs")):
        if "target/" in str(rs_file):
            continue
        mod_name = module_name_from_path(rs_file)
        text = rs_file.read_text(errors="replace")
        for m in ALL_SUBJECTS_ARRAY_RE.finditer(text):
            body = m.group(1)
            subjects = re.findall(r'"([^"]+)"', body)
            if subjects:
                if mod_name not in result:
                    result[mod_name] = []
                for s in subjects:
                    if looks_like_nats_subject(s) and s not in result[mod_name]:
                        result[mod_name].append(s)
    return result


# ── Step 6: Detect production event types from enum ──────────────────────────

PRODUCTION_EVENT_ENUM_RE = re.compile(
    r'Self::\w+\s*=>\s*"(production\.[^"]+)"'
)

def scan_production_events():
    """Returns list[subject] for production module."""
    subjects = []
    for rs_file in sorted(MODULES_DIR.glob("production/src/**/*.rs")):
        if "target/" in str(rs_file):
            continue
        text = rs_file.read_text(errors="replace")
        for m in PRODUCTION_EVENT_ENUM_RE.finditer(text):
            s = m.group(1)
            if s not in subjects:
                subjects.append(s)
    return subjects


# ── Step 7: Build final catalog ───────────────────────────────────────────────

def derive_wire_subject(event_type: str, module_prefix_map: dict[str, str], publisher: str) -> str:
    """
    Convert an event_type (as stored in outbox) to the NATS wire subject.
    For modules with a custom publisher that prepends a prefix, the wire subject
    is {prefix}.{event_type}.  For others it's the event_type directly.
    """
    if publisher and publisher in module_prefix_map:
        prefix = module_prefix_map[publisher]
        return f"{prefix}.{event_type}"
    # Some modules have known prefix conventions in their outbox code
    # even if they don't show up via FORMAT_EVENTS_PREFIX_RE
    known_prefixes = {
        "ar": "ar.events",
        "inventory": "inventory.events",
        "payments": "payments.events",
        "subscriptions": "subscriptions.events",
        "treasury": "treasury.events",
    }
    if publisher in known_prefixes:
        prefix = known_prefixes[publisher]
        return f"{prefix}.{event_type}"
    return event_type


def build_catalog():
    schemas = parse_json_schemas()
    module_configs = parse_module_configs()
    rust_subjects, publisher_prefixes = scan_rust_files()
    all_subjects_arrays = scan_all_subjects_arrays()
    production_subjects = scan_production_events()

    # Build prefix map early (needed by scan_module_event_types)
    known_prefixes_early = {
        "ar": "ar.events",
        "inventory": "inventory.events",
        "payments": "payments.events",
        "subscriptions": "subscriptions.events",
        "treasury": "treasury.events",
    }
    for mod, prefix in publisher_prefixes.items():
        known_prefixes_early[mod] = prefix

    module_event_types = scan_module_event_types(module_configs, known_prefixes_early)

    # Known publisher prefix conventions (hardcoded from code analysis)
    # These are modules that use format!("{prefix}.events.{event_type}")
    known_prefixes = {
        "ar": "ar.events",
        "inventory": "inventory.events",
        "payments": "payments.events",
        "subscriptions": "subscriptions.events",
        "treasury": "treasury.events",
    }
    # Merge with discovered prefixes
    for mod, prefix in publisher_prefixes.items():
        known_prefixes[mod] = prefix

    # Build module name map from dir name → module.toml name
    mod_name_map = {m["dir"]: m["name"] for m in module_configs}

    # Collect all unique subjects from all sources
    all_subjects: dict[str, dict] = {}

    def add_subject(subject: str, publisher: str = "", consumers: list[str] | None = None,
                    event_type: str = "", description: str = "",
                    payload_fields: list[str] | None = None, schema_file: str = ""):
        if not looks_like_nats_subject(subject):
            return
        if subject not in all_subjects:
            all_subjects[subject] = {
                "subject": subject,
                "publisher_module": "",
                "consumer_modules": [],
                "event_type": event_type or subject,
                "description": description,
                "payload_fields": payload_fields or [],
                "schema_file": schema_file,
            }
        entry = all_subjects[subject]
        if publisher and not entry["publisher_module"]:
            entry["publisher_module"] = publisher
        if consumers:
            for c in consumers:
                if c and c not in entry["consumer_modules"]:
                    entry["consumer_modules"].append(c)
        if event_type and not entry["event_type"]:
            entry["event_type"] = event_type
        if description and not entry["description"]:
            entry["description"] = description
        if payload_fields and not entry["payload_fields"]:
            entry["payload_fields"] = payload_fields
        if schema_file and not entry["schema_file"]:
            entry["schema_file"] = schema_file

    # 1. From JSON schemas: map event_type → wire subject, then add
    for event_type, info in schemas.items():
        publisher = info["publisher"]
        wire_subject = derive_wire_subject(event_type, known_prefixes, publisher)
        add_subject(
            subject=wire_subject,
            publisher=publisher,
            event_type=event_type,
            description=info["description"],
            payload_fields=info["payload_fields"],
            schema_file=info["schema_file"],
        )

    # 2. From Rust source subject constants (publishers + consumers)
    for subject, info in rust_subjects.items():
        publisher = info.get("publisher_module", "")
        consumers = list(info.get("consumer_modules", set()))
        add_subject(subject=subject, publisher=publisher, consumers=consumers)

    # 3. From ALL_SUBJECTS arrays (maintenance, workflow)
    for mod_name, subjects in all_subjects_arrays.items():
        for subject in subjects:
            add_subject(subject=subject, publisher=mod_name)

    # 4. From production events enum
    for subject in production_subjects:
        add_subject(subject=subject, publisher="production")

    # 5. From module-local EVENT_TYPE_* constants (schema-less publishers)
    # Only add if not already covered by a JSON schema entry
    schema_derived_subjects = set(all_subjects.keys())
    for wire_subject, publisher, event_type in module_event_types:
        if wire_subject not in schema_derived_subjects:
            add_subject(subject=wire_subject, publisher=publisher, event_type=event_type)

    # 7. From module.toml [events.subscribe] sections
    for mod in module_configs:
        for subj in mod["subscribed_subjects"]:
            if looks_like_nats_subject(subj):
                add_subject(subject=subj, consumers=[mod["name"]])

    # Post-process: normalize consumer_modules lists, remove publisher from consumers
    for subject, entry in all_subjects.items():
        publisher = entry["publisher_module"]
        consumers = sorted(set(c for c in entry["consumer_modules"] if c and c != publisher))
        entry["consumer_modules"] = consumers

    # Determine event_type from subjects without one
    for subject, entry in all_subjects.items():
        if not entry["event_type"]:
            entry["event_type"] = subject

    # Infer publisher from subject prefix for subjects without a known publisher
    prefix_to_module = {
        "ar.": "ar",
        "ap.": "ap",
        "gl.": "gl",
        "inventory.": "inventory",
        "payments.": "payments",
        "maintenance.": "maintenance",
        "workflow.": "workflow",
        "production.": "production",
        "shipping_receiving.": "shipping-receiving",
        "sr.": "shipping-receiving",
        "notifications.": "notifications",
        "subscriptions.": "subscriptions",
        "treasury.": "treasury",
        "timekeeping.": "timekeeping",
        "workforce_competence.": "workforce-competence",
        "party.": "party",
        "ttp.": "ttp",
        "integrations.": "integrations",
        "numbering.": "numbering",
        "smoke_test.": "smoke-test",
        "sales.": "sales",           # external — not a platform module
        "tax.": "unknown",           # external / legacy
        "docmgmt.": "unknown",
        "fa_depreciation_run.": "fixed-assets",
    }
    for subject, entry in all_subjects.items():
        if not entry["publisher_module"]:
            for prefix, mod in prefix_to_module.items():
                if subject.startswith(prefix):
                    entry["publisher_module"] = mod
                    break

    return all_subjects


# ── Step 8: Render outputs ────────────────────────────────────────────────────

def render_catalog_json(catalog: dict) -> str:
    entries = sorted(catalog.values(), key=lambda e: e["subject"])
    output = {
        "$schema": "https://7dsolutions.io/schemas/event-catalog.v1.json",
        "generated_by": "tools/event-catalog/generate.py",
        "subjects": entries,
    }
    return json.dumps(output, indent=2, sort_keys=False) + "\n"


def render_catalog_md(catalog: dict) -> str:
    entries = sorted(catalog.values(), key=lambda e: (e["publisher_module"], e["subject"]))

    # Group by publisher module
    by_module: dict[str, list] = defaultdict(list)
    for entry in entries:
        pub = entry["publisher_module"] or "unknown"
        by_module[pub].append(entry)

    lines = [
        "# Event Catalog — 7D Solutions Platform",
        "",
        "> **Generated** by `tools/event-catalog/generate.py` — do not edit manually.",
        "> Run `python3 tools/event-catalog/generate.py` to regenerate.",
        "",
        f"**{len(catalog)} total subjects** across {len(by_module)} publishing modules.",
        "",
        "## Contents",
        "",
    ]

    for mod in sorted(by_module.keys()):
        count = len(by_module[mod])
        anchor = mod.replace("-", "").replace("_", "").lower()
        lines.append(f"- [{mod}](#{anchor}) — {count} subject{'s' if count != 1 else ''}")

    lines += ["", "---", ""]

    for mod in sorted(by_module.keys()):
        anchor = mod.replace("-", "").replace("_", "").lower()
        lines += [
            f"## {mod}",
            "",
            "| Subject | Description | Consumers | Payload Fields | Schema |",
            "|---------|-------------|-----------|----------------|--------|",
        ]
        for entry in sorted(by_module[mod], key=lambda e: e["subject"]):
            subject = entry["subject"]
            desc = entry["description"] or "—"
            consumers = ", ".join(sorted(entry["consumer_modules"])) or "—"
            fields = ", ".join(entry["payload_fields"][:6]) or "—"
            if len(entry["payload_fields"]) > 6:
                fields += ", …"
            schema_file = entry["schema_file"]
            if schema_file:
                schema_link = f"[schema]({schema_file})"
            else:
                schema_link = "—"
            lines.append(
                f"| `{subject}` | {desc} | {consumers} | {fields} | {schema_link} |"
            )
        lines += [""]

    lines += [
        "---",
        "",
        "## How to add a new event",
        "",
        "1. Implement the outbox entry in your module (Guard → Mutation → Outbox pattern).",
        "2. Add a JSON Schema contract to `contracts/events/<module>-<event>.v1.json`.",
        "3. Run `python3 tools/event-catalog/generate.py` to regenerate the catalog.",
        "4. Commit both the contract file and the updated catalog.",
        "",
        "CI will fail if the catalog is out of date with the committed contracts.",
    ]

    return "\n".join(lines) + "\n"


# ── Main ──────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="Generate event catalog")
    parser.add_argument("--check", action="store_true",
                        help="Exit 1 if generated output differs from committed files")
    args = parser.parse_args()

    print("Scanning codebase...", file=sys.stderr)
    catalog = build_catalog()
    print(f"Found {len(catalog)} unique NATS subjects.", file=sys.stderr)

    json_output = render_catalog_json(catalog)
    md_output = render_catalog_md(catalog)

    if args.check:
        errors = []
        for path, generated in [(CATALOG_JSON, json_output), (CATALOG_MD, md_output)]:
            if not path.exists():
                errors.append(f"MISSING: {path.relative_to(ROOT)}")
                continue
            committed = path.read_text()
            if committed != generated:
                errors.append(f"STALE: {path.relative_to(ROOT)} — run generate.py to update")
        if errors:
            for e in errors:
                print(f"ERROR: {e}", file=sys.stderr)
            sys.exit(1)
        print("OK: event catalog is up to date.", file=sys.stderr)
    else:
        CATALOG_JSON.write_text(json_output)
        CATALOG_MD.write_text(md_output)
        print(f"Written: {CATALOG_JSON.relative_to(ROOT)}", file=sys.stderr)
        print(f"Written: {CATALOG_MD.relative_to(ROOT)}", file=sys.stderr)


if __name__ == "__main__":
    main()
