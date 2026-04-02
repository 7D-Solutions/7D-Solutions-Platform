#!/usr/bin/env python3
"""
Fix OpenAPI JSON specs by extracting response body types from utoipa annotations
in handler source files. This avoids needing to compile and run openapi_dump binaries.

Usage: python3 scripts/fix-openapi-specs.py
"""

import json
import os
import re
import glob
import sys

# Map client crate name → module handler directory
MODULE_MAP = {
    'ap': 'modules/ap/src/http',
    'ar': 'modules/ar/src/http',
    'bom': 'modules/bom/src/http',
    'consolidation': 'modules/consolidation/src/http',
    'fixed-assets': 'modules/fixed-assets/src/http',
    'gl': 'modules/gl/src/http',
    'integrations': 'modules/integrations/src/http',
    'inventory': 'modules/inventory/src/http',
    'notifications': 'modules/notifications/src/http',
    'numbering': 'modules/numbering/src/http',
    'party': 'modules/party/src/http',
    'payments': 'modules/payments/src/http',
    'platform-client-doc-mgmt': 'modules/doc-mgmt/src/http',
    'platform-client-tenant-registry': 'platform/control-plane/src/http',
    'production': 'modules/production/src/http',
    'reporting': 'modules/reporting/src/http',
    'shipping-receiving': 'modules/shipping-receiving/src/http',
    'treasury': 'modules/treasury/src/http',
    'workflow': 'modules/workflow/src/http',
}


def rust_type_to_schema(body_type, existing_schemas):
    """Convert a Rust body type from utoipa annotation to OpenAPI schema."""
    body_type = body_type.strip()

    # serde_json::Value → free-form object
    if body_type == 'serde_json::Value':
        return {"type": "object", "additionalProperties": True}

    # Vec<SomeType> → array
    vec_match = re.match(r'^Vec<(.+)>$', body_type)
    if vec_match:
        inner = vec_match.group(1).strip()
        inner_schema = rust_type_to_schema(inner, existing_schemas)
        return {"type": "array", "items": inner_schema}

    # PaginatedResponse<SomeType>
    pag_match = re.match(r'^PaginatedResponse<(.+)>$', body_type)
    if pag_match:
        inner = pag_match.group(1).strip()
        inner_name = extract_type_name(inner)
        ref_name = f"PaginatedResponse_{inner_name}"
        return {"$ref": f"#/components/schemas/{ref_name}"}

    # DataResponse<SomeType>
    data_match = re.match(r'^DataResponse<(.+)>$', body_type)
    if data_match:
        inner = data_match.group(1).strip()
        inner_name = extract_type_name(inner)
        ref_name = f"DataResponse_{inner_name}"
        return {"$ref": f"#/components/schemas/{ref_name}"}

    # Named type (possibly with crate path)
    type_name = extract_type_name(body_type)

    # Check if type exists in existing schemas
    if type_name in existing_schemas:
        return {"$ref": f"#/components/schemas/{type_name}"}

    # For unknown types, generate a free-form object
    # This handles types that derive ToSchema but aren't registered yet
    return {"$ref": f"#/components/schemas/{type_name}"}


def extract_type_name(rust_type):
    """Extract the simple type name from a potentially fully-qualified Rust path."""
    # crate::domain::bills::VendorBillWithLines → VendorBillWithLines
    # super::types::SomeType → SomeType
    parts = rust_type.split('::')
    return parts[-1].strip()


def parse_utoipa_annotations(handler_dir):
    """Parse all utoipa::path annotations from handler .rs files."""
    annotations = []

    rs_files = glob.glob(os.path.join(handler_dir, '*.rs'))
    # Also check subdirectories (e.g., webhooks/)
    rs_files += glob.glob(os.path.join(handler_dir, '**/*.rs'), recursive=True)

    for rs_file in sorted(set(rs_files)):
        with open(rs_file) as f:
            content = f.read()

        # Find all #[utoipa::path(...)] annotations
        # They can span multiple lines, so we need to handle brackets
        i = 0
        while i < len(content):
            idx = content.find('#[utoipa::path(', i)
            if idx == -1:
                break

            # Find the matching closing ]]
            start = idx
            depth = 0
            j = idx + len('#[utoipa::path(')
            depth = 1  # We're inside the first (
            bracket_depth = 1  # [ depth
            in_string = False

            while j < len(content) and (depth > 0 or bracket_depth > 0):
                c = content[j]
                if c == '"' and content[j-1:j] != '\\':
                    in_string = not in_string
                if not in_string:
                    if c == '(':
                        depth += 1
                    elif c == ')':
                        depth -= 1
                    elif c == '[':
                        bracket_depth += 1
                    elif c == ']':
                        bracket_depth -= 1
                j += 1

            annotation_text = content[start:j]
            parsed = parse_single_annotation(annotation_text, rs_file)
            if parsed:
                annotations.append(parsed)

            i = j

    return annotations


def parse_single_annotation(text, source_file):
    """Parse a single #[utoipa::path(...)] annotation."""
    # Normalize whitespace
    normalized = ' '.join(text.split())

    # Extract method
    method_match = re.search(r'#\[utoipa::path\(\s*(get|post|put|patch|delete)', normalized, re.IGNORECASE)
    if not method_match:
        return None
    method = method_match.group(1).lower()

    # Extract path
    path_match = re.search(r'path\s*=\s*"([^"]+)"', normalized)
    if not path_match:
        return None
    path = path_match.group(1)

    # Extract all response entries
    responses = []
    # Find the responses(...) block
    resp_match = re.search(r'responses\s*\(', normalized)
    if not resp_match:
        return None

    # Extract individual response entries
    resp_start = resp_match.end()
    # Find all (status = NNN, ...) entries
    resp_entries = re.finditer(
        r'\(\s*status\s*=\s*(\d+)\s*,\s*description\s*=\s*"([^"]*)"(?:\s*,\s*body\s*=\s*([^,\)]+))?\s*\)',
        normalized[resp_start:]
    )

    for entry in resp_entries:
        status = entry.group(1)
        desc = entry.group(2)
        body = entry.group(3).strip() if entry.group(3) else None
        responses.append({
            'status': status,
            'description': desc,
            'body': body,
        })

    return {
        'method': method,
        'path': path,
        'responses': responses,
        'source': source_file,
    }


def fix_spec(client_name, spec_file, handler_dir):
    """Fix a single client's OpenAPI spec."""
    if not os.path.exists(spec_file):
        return 0
    if not os.path.exists(handler_dir):
        print(f"  WARN: handler dir not found: {handler_dir}")
        return 0

    with open(spec_file) as f:
        spec = json.load(f)

    existing_schemas = set(spec.get('components', {}).get('schemas', {}).keys())
    annotations = parse_utoipa_annotations(handler_dir)

    # Build lookup: (method, path) → responses
    anno_lookup = {}
    for anno in annotations:
        key = (anno['method'], anno['path'])
        anno_lookup[key] = anno['responses']

    fixes = 0
    for path, methods in spec.get('paths', {}).items():
        for method, detail in methods.items():
            responses = detail.get('responses', {})
            for code in ['200', '201']:
                if code not in responses:
                    continue

                resp = responses[code]
                content = resp.get('content', {})
                schema = content.get('application/json', {}).get('schema', {}) if content else {}

                # Skip if already has a valid schema
                if schema and schema != {}:
                    continue

                # Look up the body type from utoipa annotation
                key = (method, path)
                if key not in anno_lookup:
                    continue

                anno_responses = anno_lookup[key]
                body_type = None
                for ar in anno_responses:
                    if ar['status'] == code and ar['body']:
                        body_type = ar['body']
                        break

                if not body_type:
                    # Check if any response has a body (might be different status)
                    for ar in anno_responses:
                        if ar['body'] and ar['status'] in ['200', '201']:
                            body_type = ar['body']
                            break

                if not body_type:
                    continue

                # Generate the schema
                new_schema = rust_type_to_schema(body_type, existing_schemas)

                # Update the spec
                if 'content' not in resp:
                    resp['content'] = {}
                resp['content']['application/json'] = {'schema': new_schema}
                fixes += 1
                print(f"  FIXED: {method.upper()} {path} [{code}] → {body_type}")

    if fixes > 0:
        with open(spec_file, 'w') as f:
            json.dump(spec, f, indent=2)
            f.write('\n')

    return fixes


def main():
    total_fixes = 0

    for client_name in sorted(MODULE_MAP.keys()):
        spec_file = f"clients/{client_name}/openapi.json"
        handler_dir = MODULE_MAP[client_name]

        if not os.path.exists(spec_file):
            continue

        print(f"\n{client_name}:")
        fixes = fix_spec(client_name, spec_file, handler_dir)
        if fixes:
            print(f"  → {fixes} responses fixed")
        else:
            print(f"  → no fixes needed")
        total_fixes += fixes

    print(f"\n{'='*60}")
    print(f"Total: {total_fixes} response schemas fixed across all specs")


if __name__ == '__main__':
    main()
