-- Copy to ~/.config/rockdown/config.lua, or pass --config PATH.
-- User-level TOML takes precedence when both files exist.
return {
  font_family = "Menlo",
  font_size = 15,
  line_height = 30,
  explorer_width = 290,
  terminal_height = 240,
  -- Ctrl-E toggles the right dock; drag its left edge to resize.
  theme = {
    background = "#171b22",
    panel = "#1e242e",
    foreground = "#dce3ec",
    muted = "#8995a7",
    accent = "#9cc7b5",
  },
  keys = {
    ["cmd-s"] = "save",
    ["ctrl-s"] = "save",
    ["cmd-v"] = "paste",
    ["cmd-e"] = "explorer",
    ["ctrl-e"] = "explorer",
    ["ctrl-`"] = "terminal",
    ["cmd-1"] = "editor",
    ["ctrl-pageup"] = "previous-buffer",
    ["ctrl-pagedown"] = "next-buffer",
    ["cmd-w"] = "buffer-delete",
    ["ctrl-shift-w"] = "buffer-delete",
    f1 = "help",
  },
}
