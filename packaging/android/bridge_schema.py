"""Shared bridge-contract core: error type, guarded checks, comment stripping.

Imported by generate/verify/test scripts so the schema helpers live once.
Unlike Python assert, checks survive python -O.
"""
import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True


class ContractError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise ContractError(message)


def uncomment(text):
    # Preserve strings containing comment-like characters, including
    # Kotlin raw strings and Rust raw strings.
    return re.sub(r'"""[\s\S]*?"""|r(#+)"[\s\S]*?"\1|"(?:\\.|[^"\\])*"|//[^\n]*|/\*.*?\*/',
                  lambda m: m[0] if m[0][0] in '"r' else " ", text, flags=re.DOTALL)


def repo_root():
    return Path(__file__).resolve().parents[2]
