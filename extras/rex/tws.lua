-- Opens tws in the Rex app. Load it from ~/.config/rex/init.lua with
-- pcall(dofile, ...), then run `rex config reload`. install.sh does both.
--
-- A key press runs this as the app's own action, so it works with Remote
-- Control off. Only actions sent from the CLI need Remote Control.

-- Change the key here.
local KEY = "cmd+s"

local T = "com.superlogical.terminal"
-- The shell runs without .zshrc here, and that is often where PATH gets
-- ~/.local/bin, the install.sh directory.
local BIN = os.getenv("HOME") .. "/.local/bin"

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

-- session.view only reads an attached session, and rex.session.attach exists
-- only in `rex do` scripts. The server method works in an action too, and it
-- returns the view.
local function attach(session_id)
  local a = rex.call("session.attach", {session_id = session_id})
  return a and a.view
end

local function find_tws()
  for _, s in ipairs(rex.call("session.list").sessions) do
    local view = attach(s.session_id)
    for _, w in ipairs(view and view.windows or {}) do
      for _, l in ipairs(w.layers or {}) do
        for _, b in ipairs(l.blocks or {}) do
          if runs_tws(s.session_id, b.block_id) then
            return s.session_id, w.window_id, b.block_id
          end
        end
      end
    end
  end
end

rex.action{
  name = "tws_open",
  title = "Open tws",
  keywords = "tws threads workspace",
  run = function(ctx, args)
    local session_id, window_id, block_id = find_tws()
    if session_id ~= nil then
      rex.client.queue("session.select", {session_id = session_id, window_id = window_id})
      rex.call("session.focus_block", {session_id = session_id, block_id = block_id})
      return {session_id = session_id, block_id = block_id, created = false}
    end
    -- A popup over the tab you are in. A block cannot move to another
    -- session, so tws quits after it opens a session (--popup), and the
    -- popup closes with it. tws restores its cursor from ui-state.json.
    attach(ctx.session_id)
    local layer, err = rex.call("session.new_layer", {
      session_id = ctx.session_id,
      bounds = {0.1, 0.1, 0.8, 0.8},
      focus = true,
      layout = rex.layout.block{flavor = T .. ".shell", options = {
        command = {"tws", "--popup"}, path_prepend = {BIN}}},
    })
    if layer == nil then error(err, 0) end
    return {session_id = ctx.session_id, block_id = layer.block_ids[1], created = true}
  end,
}

rex.bind(KEY, "tws_open")
