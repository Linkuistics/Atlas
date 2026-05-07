"""Public Python module — verifies the python-surface analyser
(Phase 2 PR-3) emits at least one binding.
"""


def consume_stringable():
    """Simulates calling into ex_app's Stringable behaviour."""
    return "stringified"


class Consumer:
    """Public class binding."""

    def to_string(self) -> str:
        return consume_stringable()
