-- Game CLI - Neovim Lua Plugin
-- A simple game implementation using Vim/Neovim

local game = {}

-- Game state
game.state = {
    player_x = 25,
    player_y = 11,
    width = 80,
    height = 24,
    universe = "lobby",
    game_icons = {
        { x = 10, y = 7, char = "P", name = "PointSet", target = "pointset" },
        { x = 30, y = 7, char = "T", name = "Tetris", target = "tetris" },
        { x = 10, y = 14, char = "S", name = "Snake", target = "snake" },
        { x = 30, y = 14, char = "I", name = "Invaders", target = "invaders" },
    }
}

-- Initialize the game
function game.init()
    -- Set up display options
    vim.cmd("set number!")
    vim.cmd("set relativenumber!")
    vim.cmd("set cursorline!")
    vim.cmd("set nowrap")

    -- Create a new buffer
    vim.cmd("enew")
    vim.cmd("file GameLobby")

    local buf = vim.api.nvim_get_current_buf()

    -- Set buffer options
    vim.api.nvim_buf_set_option(buf, "buftype", "nofile")
    vim.api.nvim_buf_set_option(buf, "bufhidden", "wipe")
    vim.api.nvim_buf_set_option(buf, "swapfile", false)
    vim.api.nvim_buf_set_option(buf, "modifiable", true)

    -- Set key mappings
    local opts = { buffer = buf, noremap = true, silent = true }
    vim.keymap.set("n", "h", function() game.move("left") end, opts)
    vim.keymap.set("n", "j", function() game.move("down") end, opts)
    vim.keymap.set("n", "k", function() game.move("up") end, opts)
    vim.keymap.set("n", "l", function() game.move("right") end, opts)
    vim.keymap.set("n", "<Left>", function() game.move("left") end, opts)
    vim.keymap.set("n", "<Down>", function() game.move("down") end, opts)
    vim.keymap.set("n", "<Up>", function() game.move("up") end, opts)
    vim.keymap.set("n", "<Right>", function() game.move("right") end, opts)
    vim.keymap.set("n", "q", ":quit<CR>", opts)

    -- Initial render
    game.render()
end

-- Render the game board
function game.render()
    local lines = {}

    -- Title
    local title = "╔═══════════════════════════════════════════════════════════════════════════════╗"
    table.insert(lines, title)

    local title2 = "║                        ⚙️  CHOOSE YOUR GAME  ⚙️                              ║"
    table.insert(lines, title2)

    local sep = "╠═══════════════════════════════════════════════════════════════════════════════╣"
    table.insert(lines, sep)

    -- Draw board (rows 4-22)
    for y = 1, game.state.height - 4 do
        local line = "║ "

        for x = 1, game.state.width - 4 do
            local char = " "

            -- Check for game icons
            for _, icon in ipairs(game.state.game_icons) do
                if x == icon.x and y == icon.y then
                    char = icon.char
                    break
                end
            end

            -- Check for player
            if x == game.state.player_x and y == game.state.player_y then
                char = "@"
            end

            line = line .. char
        end

        line = line .. " ║"
        table.insert(lines, line)
    end

    local bottom = "╠═══════════════════════════════════════════════════════════════════════════════╣"
    table.insert(lines, bottom)

    local help1 = "║ hjkl/arrows: move   q: quit   P/T/S/I: Games                                 ║"
    local help2 = "║ Step on a game icon to enter (when implemented)                                ║"
    table.insert(lines, help1)
    table.insert(lines, help2)

    local end_line = "╚═══════════════════════════════════════════════════════════════════════════════╝"
    table.insert(lines, end_line)

    -- Set buffer content
    local buf = vim.api.nvim_get_current_buf()
    vim.api.nvim_buf_set_option(buf, "modifiable", true)
    vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
    vim.api.nvim_buf_set_option(buf, "modifiable", false)
end

-- Move player
function game.move(direction)
    local new_x = game.state.player_x
    local new_y = game.state.player_y

    if direction == "up" then
        new_y = math.max(2, new_y - 1)
    elseif direction == "down" then
        new_y = math.min(game.state.height - 4, new_y + 1)
    elseif direction == "left" then
        new_x = math.max(2, new_x - 1)
    elseif direction == "right" then
        new_x = math.min(game.state.width - 6, new_x + 1)
    end

    -- Update position
    game.state.player_x = new_x
    game.state.player_y = new_y

    -- Check if on icon
    for _, icon in ipairs(game.state.game_icons) do
        if new_x == icon.x and new_y == icon.y then
            vim.notify("📍 Entered " .. icon.name .. "! (Coming soon)", vim.log.levels.INFO)
        end
    end

    -- Redraw
    game.render()
end

-- Start the game
game.init()

return game
