-- Opens tws in the Rex app. Load it from ~/.config/rex/init.lua with
-- pcall(dofile, ...), then run `rex config reload`. install.sh does both.
--
-- A key press runs this as the app's own action, so it works with Remote
-- Control off. Only actions sent from the CLI need Remote Control.

-- Change the key here.
local KEY = "ctrl+cmd+t"

local T = "com.superlogical.terminal"
-- `_` keeps this session out of the tws tree: tws ignores such labels.
local HOME_LABEL = "tws_home"

local function basename(path)
  return path and path:match("([^/]+)$")
end

-- The process tells a running tws from a tab that only has the name, and it
-- finds a tws that the user started by hand in any tab. Two tws instances
-- would overwrite each other's state.json.
local function runs_tws(session_id, block_id)
  local p = rex.block.call(T, "process", {block_id = block_id, session_id = session_id})
  local fg = p and p.foreground
  return fg ~= nil and (basename(fg.argv0) == "tws" or basename(fg.path) == "tws")
end

local function find_tws()
  local home
  for _, s in ipairs(rex.call("session.list").sessions) do
    if s.label == HOME_LABEL then home = s.session_id end
    rex.session.attach{session_id = s.session_id}
    local view = rex.session.view{session_id = s.session_id}
    for _, w in ipairs(view and view.windows or {}) do
      for _, l in ipairs(w.layers or {}) do
        for _, b in ipairs(l.blocks or {}) do
          if runs_tws(s.session_id, b.block_id) then
            return s.session_id, w.window_id, home
          end
        end
      end
    end
  end
  return nil, nil, home
end

-- The shell runs without .zshrc here, and that is often where PATH gets
-- ~/.local/bin, the install.sh directory.
local function tws_layout()
  return rex.layout.block{flavor = T .. ".shell", options = {command = {"tws"},
    path_prepend = {os.getenv("HOME") .. "/.local/bin"}}}
end

rex.action{
  name = "tws_open",
  title = "Open tws",
  keywords = "tws threads workspace",
  run = function(ctx, args)
    local session_id, window_id, home = find_tws()
    local created = false
    if session_id == nil then
      created = true
      -- tws closes its tab when it quits, and the empty session stays.
      -- Reuse it, so each press does not add one more.
      if home ~= nil then
        local w, err = rex.session.new_window{session_id = home, window_label = "tws",
          layout = tws_layout()}
        if w == nil then error(err, 0) end
        session_id, window_id = home, w.window_id
      else
        local s, err = rex.call("session.create", {label = HOME_LABEL,
          initial_windows = {{window_label = "tws", layout = tws_layout()}}})
        if s == nil then error(err, 0) end
        session_id, window_id = s.session_id, s.initial_windows[1].window_id
      end
    end
    rex.client.queue("session.select", {session_id = session_id, window_id = window_id})
    return {session_id = session_id, window_id = window_id, created = created}
  end,
}

rex.bind(KEY, "tws_open")
