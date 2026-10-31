"""Colour themes: Utsuri's own blue glass, plus seven dark schemes from the Ryoku colour
scheme catalogue (github.com/CratusGod/ryostore, imported from noctalia-colorschemes,
MIT). Each scheme's primary/secondary/tertiary become the accents and the glass rim,
and its surface colour tints the glass.

Call sites keep their own tint alphas and shades: tint() moves them by the same offset
from Default's base glass colour, so Default looks exactly as before.
"""
from __future__ import annotations

from dataclasses import dataclass

from PySide6.QtGui import QColor

DEFAULT = "Default"
_BASE = (22, 18, 34)  # Default's glass colour; call sites tint relative to it


@dataclass(frozen=True)
class Theme:
    name: str
    accent_a: str        # gradients run a -> b -> c (primary buttons, sliders, checkboxes)
    accent_b: str
    accent_c: str
    surface: str         # tints the glass
    surface_variant: str  # menus, tooltips, drop-down lists
    text: str
    text_dim: str
    check_a: str         # a toggled button (e.g. Text / Image)
    check_b: str
    rim: tuple[str, ...]  # the glass rim's colour ring; first == last


def _ryoku(name, primary, secondary, tertiary, error, surface, variant, on_surface, on_variant) -> Theme:
    return Theme(name, primary, secondary, tertiary, surface, variant, on_surface, on_variant,
                 primary, error, (primary, secondary, tertiary, error, primary))


THEMES: dict[str, Theme] = {t.name: t for t in [
    Theme(DEFAULT, "#5ac8ff", "#2f7bff", "#5b5bff", "#161222", "#2a2438", "#ffffff",
          "rgba(255,255,255,0.68)", "rgba(255,138,92,0.85)", "rgba(255,79,139,0.85)",
          ("#ff6b9d", "#ffb36b", "#ffe66b", "#7dffb0", "#6bd5ff", "#8a7bff", "#e07bff", "#ff6b9d")),
    # name, primary, secondary, tertiary, error, surface, surface variant, on surface, on variant
    _ryoku("Catppuccin Macchiato", "#b7bdf8", "#f0c6c6", "#8bd5ca", "#ed8796",
           "#24273a", "#363a4f", "#cad3f5", "#b8c0e0"),
    _ryoku("Tokyo Night Storm", "#7aa2f7", "#bb9af7", "#73daca", "#f7768e",
           "#24283b", "#343b58", "#a9b1d6", "#9aa5ce"),
    _ryoku("Gruvbox Dark Hard", "#83a598", "#b8bb26", "#8ec07c", "#fb4934",
           "#1d2021", "#3c3836", "#ebdbb2", "#d5c4a1"),
    _ryoku("Rosé Pine Moon", "#ea9a97", "#9ccfd8", "#3e8fb0", "#eb6f92",
           "#232136", "#393552", "#e0def4", "#908caa"),
    _ryoku("Everforest Dark Hard", "#a7c080", "#7fbbb3", "#83c092", "#e67e80",
           "#1e2326", "#2e383c", "#d3c6aa", "#9da9a0"),
    _ryoku("Kanagawa Dragon", "#8a9a7b", "#8ea4a2", "#c4746e", "#c4746e",
           "#181616", "#282727", "#c5c9c5", "#c8c093"),
    _ryoku("Synthwave 84", "#ff536c", "#7f9fff", "#c080ff", "#ff536c",
           "#18191f", "#2a2b32", "#c4c4d6", "#c4c4d6"),
]}

_current = THEMES[DEFAULT]


def names() -> list[str]:
    return list(THEMES)


def current() -> Theme:
    return _current


def set_theme(name: str) -> Theme:
    """Switch themes; an unknown name (an old or hand-edited setting) means Default."""
    global _current
    _current = THEMES.get(name, THEMES[DEFAULT])
    return _current


def _rgb(hex_color: str) -> tuple[int, int, int]:
    """"#rrggbb" -> (r, g, b), in plain Python: liquid.render calls this from worker
    threads, where Qt objects (even a QColor) must not be made (it crashed on macOS)."""
    h = hex_color.lstrip("#")
    return int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16)


def tint(r: int, g: int, b: int, a: int) -> QColor:
    """The glass colour a call site asked for, moved onto this theme's surface."""
    if _current.name != DEFAULT:
        surface = _rgb(_current.surface)
        r, g, b = (max(0, min(255, s + v - o)) for s, v, o in zip(surface, (r, g, b), _BASE))
    return QColor(r, g, b, a)


def liquid_tint() -> tuple[int, int, int, float]:
    """The liquid renderer's tint on dark backdrops. Mostly the theme's surface, with
    enough of its accent that the glass itself shows which theme is on."""
    if _current.name == DEFAULT:
        return (18, 14, 30, 0.26)
    s, a = _rgb(_current.surface), _rgb(_current.accent_a)
    r, g, b = (round(x + (y - x) * ACCENT_MIX) for x, y in zip(s, a))
    return (r, g, b, 0.36)


ACCENT_MIX = 0.38  # how much of the accent goes into the glass tint


def rim_colors() -> tuple[tuple[int, int, int], tuple[int, int, int]]:
    """The liquid glass's edge light: (side facing the light, far side). White by
    default; a theme lights its edges with its accent and secondary colours."""
    if _current.name == DEFAULT:
        return (255, 255, 255), (255, 255, 255)
    a, b = _rgb(_current.accent_a), _rgb(_current.accent_b)
    lit = tuple(round(255 + (x - 255) * 0.55) for x in a)
    return lit, b


def label_rgb() -> tuple[int, int, int]:
    """Menu label colour on dark glass: white by default, the theme's text colour otherwise."""
    return (255, 255, 255) if _current.name == DEFAULT else _rgb(_current.text)


def contrast(fg: str, bg: str) -> float:
    """WCAG contrast ratio of two opaque colours."""
    def lum(hex_color):
        def ch(v):
            v /= 255
            return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4
        r, g, b = _rgb(hex_color)
        return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
    a, b = sorted((lum(fg), lum(bg)), reverse=True)
    return (a + 0.05) / (b + 0.05)
