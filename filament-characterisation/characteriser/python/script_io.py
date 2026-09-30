"""Reads a script's input from stdin and writes its output to stdout, as JSON.

The input is validated against the schema generated from the Rust types, in
`schemas/`, so a script never works on input of a shape it doesn't expect.
"""

import json
import sys
from pathlib import Path
from typing import Any

import jsonschema

SCHEMAS_DIR = Path(__file__).parent / "schemas"


def read_input(script_name: str) -> Any:
    """Reads the input from stdin and validates it against `script_name`'s
    input schema, exiting with the error on stderr if either fails.

    `script_name` is the script's file name without `.py`.
    """
    schema = json.loads((SCHEMAS_DIR / f"{script_name}.input.json").read_text())

    try:
        data = json.load(sys.stdin)
    except json.JSONDecodeError as e:
        print(f"couldn't parse the input: {e}", file=sys.stderr)
        sys.exit(1)

    try:
        jsonschema.validate(data, schema)
    except jsonschema.ValidationError as e:
        print(f"invalid input: {e.message}", file=sys.stderr)
        sys.exit(1)

    return data


def write_output(output: Any) -> None:
    """Writes `output` to stdout.

    By default Python writes NaN and infinities, which aren't valid JSON and
    which Rust would reject, so this fails in Python with a clear message
    instead.
    """
    json.dump(output, sys.stdout, allow_nan=False)
