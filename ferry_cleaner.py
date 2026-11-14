#!/usr/bin/env python3
"""Thin root wrapper re-exporting integrations/antigravity/ferry_cleaner.py."""

import importlib.util
import os


def _load_impl():
    impl_path = os.path.join(
        os.path.dirname(os.path.abspath(__file__)),
        "integrations",
        "antigravity",
        "ferry_cleaner.py",
    )
    spec = importlib.util.spec_from_file_location("_ferry_antigravity_cleaner", impl_path)
    if spec is None or spec.loader is None:
        raise ImportError(f"Unable to load {impl_path}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


_impl = _load_impl()
for _name, _val in vars(_impl).items():
    if not _name.startswith("_") or _name == "_clean_text_chunk":
        globals()[_name] = _val


def main() -> None:
    _impl.main(script_path=os.path.abspath(__file__))


if __name__ == "__main__":
    main()
