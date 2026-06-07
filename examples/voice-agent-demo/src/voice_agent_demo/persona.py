"""Persona loader.

Each persona JSON describes one person + the agent that represents them
on the relay. Editing the JSON is the documented way to retune what an
agent negotiates with — the negotiation prompt treats the lists as
opaque ranked inputs, so adding/reordering items just works.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

PERSONAS_DIR = Path(__file__).resolve().parent.parent.parent / "personas"


@dataclass(frozen=True)
class Persona:
    """One person + their agent's negotiating ingredients."""

    name: str  # internal id, also the persona JSON filename
    display_name: str
    agent_slug: str
    agent_display_name: str
    agent_description: str
    drinks_ranked: list[str]
    food_ranked: list[str]
    dietary_notes: str
    voice_speaker: str = ""  # Sarvam TTS speaker; falls back to env/default

    @classmethod
    def load(cls, name: str) -> "Persona":
        path = PERSONAS_DIR / f"{name}.json"
        if not path.exists():
            available = sorted(p.stem for p in PERSONAS_DIR.glob("*.json"))
            raise FileNotFoundError(
                f"no persona named {name!r}; available: {available}"
            )
        data = json.loads(path.read_text())
        # Defensive: a persona with empty rankings would silently produce
        # a dull negotiation; fail loudly at load instead.
        for k in ("drinks_ranked", "food_ranked"):
            if not data.get(k):
                raise ValueError(f"persona {name}: {k} must be a non-empty list")
        return cls(
            name=name,
            display_name=data["display_name"],
            agent_slug=data["agent_slug"],
            agent_display_name=data["agent_display_name"],
            agent_description=data.get("agent_description", ""),
            drinks_ranked=list(data["drinks_ranked"]),
            food_ranked=list(data["food_ranked"]),
            dietary_notes=data.get("dietary_notes", ""),
            voice_speaker=data.get("voice_speaker", ""),
        )

    def negotiation_payload(self) -> dict:
        """Ranked prefs in the shape the `negotiate_dinner` tool expects.

        Kept narrow on purpose — the receiving agent's LLM should reason
        over rank order, not get distracted by free-form notes.
        """
        return {
            "person": self.display_name,
            "drinks_ranked": self.drinks_ranked,
            "food_ranked": self.food_ranked,
            "dietary_notes": self.dietary_notes,
        }
