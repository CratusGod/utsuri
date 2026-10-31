"""Colour themes: every scheme is complete and readable, Default is unchanged, and a
change in Settings restyles open windows."""
from __future__ import annotations

from dataclasses import fields

import pytest
from PySide6.QtGui import QColor

from utsuri import glass, themes


@pytest.fixture(autouse=True)
def back_to_default():
    from PySide6.QtWidgets import QApplication
    QApplication.instance() or QApplication([])  # glass.qss() asks Qt's font database
    yield
    themes.set_theme(themes.DEFAULT)


def test_there_are_seven_ryoku_themes_plus_default():
    assert themes.names()[0] == themes.DEFAULT and len(themes.names()) == 8


@pytest.mark.parametrize("name", themes.names())
def test_every_colour_is_valid(name):
    t = themes.THEMES[name]
    for f in fields(t):
        v = getattr(t, f.name)
        for c in (v if isinstance(v, tuple) else [v] if f.name != "name" else []):
            assert QColor(c).isValid() or c.startswith("rgba("), f"{name}.{f.name} = {c}"
    assert t.rim[0] == t.rim[-1], "the rim's colour ring must close"


@pytest.mark.parametrize("name", themes.names()[1:])
def test_text_and_accents_are_readable_on_the_glass(name):
    t = themes.THEMES[name]
    assert themes.contrast(t.text, t.surface) >= 4.5, "body text (WCAG AA)"
    assert themes.contrast(t.text_dim, t.surface) >= 4.5, "dim text (WCAG AA)"
    assert themes.contrast(t.accent_a, t.surface) >= 3, "accent"


def test_default_tints_are_unchanged():
    assert themes.tint(22, 17, 34, 150) == QColor(22, 17, 34, 150)
    assert themes.liquid_tint() == (18, 14, 30, 0.26)


def test_a_theme_moves_tints_onto_its_surface_keeping_the_shade():
    t = themes.set_theme("Gruvbox Dark Hard")
    base = QColor(t.surface)
    assert themes.tint(22, 18, 34, 62) == QColor(base.red(), base.green(), base.blue(), 62)
    lighter = themes.tint(38, 30, 56, 228)  # window fallback: a lighter shade than the menu's
    assert (lighter.red(), lighter.green(), lighter.blue()) == (base.red() + 16, base.green() + 12, base.blue() + 22)


def test_unknown_theme_name_falls_back_to_default():
    assert themes.set_theme("No Such Theme").name == themes.DEFAULT


@pytest.mark.parametrize("name", themes.names())
def test_stylesheet_uses_the_theme(name):
    t = themes.set_theme(name)
    qss = glass.qss()
    assert t.accent_a in qss and t.surface_variant in qss and "@FONT@" not in qss
    assert qss.count("{") == qss.count("}")


class FakeSettings:
    def __init__(self):
        self.v = {"trigger": "shift", "output": "next", "jpg_quality": 90, "pdf_dpi": 200, "theme": "Default"}

    def get(self, k):
        return self.v[k]

    def set(self, k, value):
        self.v[k] = value
        themes.set_theme(self.v["theme"])


def test_choosing_a_theme_in_settings_restyles_open_windows(monkeypatch):
    from utsuri import settings
    monkeypatch.setattr(settings, "autostart_enabled", lambda: False)
    w = settings.SettingsWindow(FakeSettings())
    assert "#5ac8ff" in w.styleSheet()
    w.theme.setCurrentIndex(themes.names().index("Tokyo Night Storm"))
    assert themes.current().name == "Tokyo Night Storm"
    assert "#7aa2f7" in w.styleSheet() and "#5ac8ff" not in w.styleSheet()
    w.grab()  # paints with the new tint
    w.close()
