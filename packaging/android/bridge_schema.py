"""Shared bridge-contract core: error type, guarded checks, comment stripping.

Imported by generate/verify/test scripts so the schema helpers live once.
Unlike Python assert, checks survive python -O.
"""
import re
from pathlib import Path


class ContractError(ValueError):
    pass


def require(condition, message="unsupported or inconsistent bridge schema"):
    if not condition:
        raise ContractError(message)


def load_sibling(module_name, file_name):
    """Import a sibling script as a module (single importlib recipe)."""
    import importlib.util
    from pathlib import Path

    spec = importlib.util.spec_from_file_location(
        module_name, Path(__file__).with_name(file_name)
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


_STRINGS = re.compile(r'"""(.*?)"""|r(#+)"(.*?)"\1|"(?:\\.|[^"\\])*"', re.DOTALL)
_COMMENTS = re.compile(r"//[^\n]*|/\*.*?\*/", re.DOTALL)


def uncomment(text):
    # Preserve strings containing comment-like characters, including
    # Kotlin raw strings and Rust raw strings. Strings are masked first
    # so comment syntax inside them survives; each pattern stays simple.
    kept = []

    def stash(match):
        kept.append(match[0])
        return f"\0{len(kept) - 1}\0"

    masked = _STRINGS.sub(stash, text)
    masked = _COMMENTS.sub(" ", masked)
    for index, literal in enumerate(kept):
        masked = masked.replace(f"\0{index}\0", literal)
    return masked


def repo_root():
    return Path(__file__).resolve().parents[2]
