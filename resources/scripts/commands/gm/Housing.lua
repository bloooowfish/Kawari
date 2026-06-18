required_rank = GM_RANK_DEBUG
command_sender = "[housing] "

local DEFAULT_KIND = "personal"
local DEFAULT_SIZE = "large"
local DEFAULT_TERRITORY_ID = 340
local DEFAULT_WARD = 1
local DEFAULT_PLOT = 6
local MAX_APARTMENT_ROOM_NUMBER = 1023

local EXTERIOR_FIELDS = {
    roof = true,
    walls = true,
    windows = true,
    door = true,
    roof_fixture = true,
    wall_fixture = true,
    above_door_banner = true,
    fence = true,
}

local INTERIOR_FIELDS = {
    window_style = true,
    door_style = true,
    door_stain = true,
    ground_walls = true,
    ground_floor = true,
    ground_chandelier = true,
    top_walls = true,
    top_floor = true,
    top_chandelier = true,
    cellar_walls = true,
    cellar_floor = true,
    cellar_chandelier = true,
}

local INTERIOR_PRESETS = {
    reference_medium_interior = {
        { "window_style", 2601 },
        { "door_style", 553 },
        { "door_stain", 365 },
        { "ground_walls", 66050 },
        { "ground_floor", 65570 },
        { "ground_chandelier", 65821 },
        { "top_walls", 66091 },
        { "top_floor", 65538 },
        { "top_chandelier", 65848 },
        { "cellar_walls", 66108 },
        { "cellar_floor", 65583 },
        { "cellar_chandelier", 65796 },
    },
}

INTERIOR_PRESETS.reference_medium = INTERIOR_PRESETS.reference_medium_interior
INTERIOR_PRESETS.retail_shirogane_medium = INTERIOR_PRESETS.reference_medium_interior
INTERIOR_PRESETS.capture = INTERIOR_PRESETS.reference_medium_interior

local function lower(value)
    if value == nil then
        return nil
    end

    return string.lower(value)
end

local function usage(player)
    printf(player, "Usage: !housing")
    printf(player, "       !housing testhouse [personal|fc] [small|medium|large] [territory_id] [ward] [plot]")
    printf(player, "       !housing apartment [room]")
    printf(player, "       !housing enter [apartment [room]]")
    printf(player, "       !housing exit")
    printf(player, "       !housing reload")
    printf(player, "       !housing info")
    printf(player, "       !housing reset furniture|estate|all")
    printf(player, "       !housing light <0-5>")
    printf(player, "       !housing greeting <text>")
    printf(player, "       !housing name <text>")
    printf(player, "       !housing exterior <field> <value>")
    printf(player, "       !housing exterior color <field> <stain>")
    printf(player, "       !housing interior <field> <value>")
    printf(player, "       !housing interior preset reference_medium_interior|reference_medium|retail_shirogane_medium|capture")
    printf(player, "       !housing preset [all|interior|indoor|exterior|outdoor] <ReMakePlace json path or preset name> [--reload]")
    printf(player, "       !housing preset latest [all|interior|indoor|exterior|outdoor] [--reload]")
    printf(player, "       !housing preset repeat [--reload]")
    printf(player, "       !housing preset check [all|interior|indoor|exterior|outdoor] <path|latest>")
    printf(player, "       !housing preset check repeat")
    printf(player, "       !housing givekit indoor|outdoor|npc")
end

local function join_args(args, start_index)
    local values = {}

    for i = start_index, #args do
        values[#values + 1] = args[i]
    end

    return table.concat(values, " ")
end

local function parse_kind(value)
    value = lower(value or DEFAULT_KIND)

    if value == "personal" or value == "fc" then
        return value
    end

    return nil
end

local function parse_size(value)
    value = lower(value or DEFAULT_SIZE)

    if value == "small" or value == "medium" or value == "large" then
        return value
    end

    return nil
end

local function parse_integer(value, default_value)
    if value == nil then
        return default_value
    end

    local parsed = tonumber(value)
    if parsed == nil or parsed ~= math.floor(parsed) then
        return nil
    end

    return parsed
end

local function parse_range(value, default_value, minimum, maximum)
    local parsed = parse_integer(value, default_value)

    if parsed == nil or parsed < minimum or parsed > maximum then
        return nil
    end

    return parsed
end

local function parse_non_negative_integer(value, maximum)
    local parsed = parse_integer(value, nil)

    if parsed == nil or parsed < 0 then
        return nil
    end

    if maximum ~= nil and parsed > maximum then
        return nil
    end

    return parsed
end

local function is_preset_scope(value)
    value = lower(value)
    return value == "all" or value == "interior" or value == "indoor" or value == "exterior" or value == "outdoor"
end

local function is_reload_flag(value)
    value = lower(value)
    return value == "--reload"
end

local function collect_preset_args(args, start_index)
    local values = {}
    local reload = false

    for i = start_index, #args do
        values[#values + 1] = args[i]
    end

    if is_reload_flag(values[#values]) then
        reload = true
        table.remove(values, #values)
    end

    return values, reload
end

local function join_values(values, start_index)
    local selected = {}

    for i = start_index, #values do
        selected[#selected + 1] = values[i]
    end

    return table.concat(selected, " ")
end

local function parse_preset_args(args)
    local values, reload = collect_preset_args(args, 2)
    local check_only = false
    local scope = "all"
    local explicit_scope = false

    if lower(values[1]) == "check" then
        check_only = true
        table.remove(values, 1)
    end

    if check_only and reload then
        return nil
    end

    if is_preset_scope(values[1]) then
        scope = values[1]
        explicit_scope = true
        table.remove(values, 1)
    end

    local source = lower(values[1])
    if source == nil then
        return nil
    end

    if source == "latest" then
        if is_preset_scope(values[2]) then
            scope = values[2]
            table.remove(values, 2)
        end

        if #values ~= 1 then
            return nil
        end

        return { kind = "latest", scope = scope, reload = reload, check_only = check_only }
    end

    if source == "repeat" then
        if #values ~= 1 or explicit_scope then
            return nil
        end

        return { kind = "repeat", scope = scope, reload = reload, check_only = check_only }
    end

    local preset_path = join_values(values, 1)

    if preset_path == "" then
        return nil
    end

    return { kind = "path", path = preset_path, scope = scope, reload = reload, check_only = check_only }
end

local function reload_suffix(reload)
    if reload then
        return " with reload"
    end

    return ""
end

local function handle_testhouse(player, args)
    local kind = parse_kind(args[2])
    local size = parse_size(args[3])
    local territory_id = parse_range(args[4], DEFAULT_TERRITORY_ID, 1, 65535)
    local ward = parse_range(args[5], DEFAULT_WARD, 1, 30)
    local plot = parse_range(args[6], DEFAULT_PLOT, 1, 30)

    if kind == nil or size == nil or territory_id == nil or ward == nil or plot == nil then
        usage(player)
        return
    end

    player:ensure_test_house_with_options(kind, size, territory_id, ward - 1, 0, plot - 1)
    printf(player, "Created or refreshed your local %s %s test estate at territory %d ward %d plot %d.", kind, size, territory_id, ward, plot)
end

local function handle_enter(player, args)
    local target = lower(args[2])

    if target == "apartment" then
        local room = parse_range(args[3], 1, 1, MAX_APARTMENT_ROOM_NUMBER)

        if room == nil then
            usage(player)
            return
        end

        player:enter_test_apartment(room)
        printf(player, "Entering your local apartment room %d.", room)
        return
    end

    player:enter_test_house()
    printf(player, "Entering your local test estate.")
end

local function handle_exit(player, args)
    player:exit_test_house()
    printf(player, "Exiting your local test estate.")
end

local function handle_reload(player, args)
    player:reload_housing()
    printf(player, "Reloading your local test estate.")
end

local function handle_info(player, args)
    local context = player:get_housing_ward_context()
    printf(player, "Housing context: territory %d ward %d division %d.", context.territory_type_id, context.ward_index + 1, context.division)
end

local function handle_reset(player, args)
    local mode = lower(args[2])

    if mode ~= "furniture" and mode ~= "estate" and mode ~= "all" then
        usage(player)
        return
    end

    player:reset_housing(mode)
    printf(player, "Queued housing reset: %s.", mode)
end

local function handle_light(player, args)
    local level = parse_range(args[2], nil, 0, 5)

    if level == nil then
        usage(player)
        return
    end

    player:update_housing_light(level)
    printf(player, "Queued housing light level update: %d.", level)
end

local function handle_greeting(player, args)
    local greeting = join_args(args, 2)

    if greeting == "" then
        usage(player)
        return
    end

    player:update_housing_greeting(greeting)
    printf(player, "Queued housing greeting update.")
end

local function handle_name(player, args)
    local estate_name = join_args(args, 2)

    if estate_name == "" then
        usage(player)
        return
    end

    player:update_housing_name(estate_name)
    printf(player, "Queued housing name update.")
end

local function handle_exterior(player, args)
    local mode = lower(args[2])

    if mode == "color" then
        local field = lower(args[3])
        local stain = parse_range(args[4], nil, 0, 255)

        if not EXTERIOR_FIELDS[field] or stain == nil then
            usage(player)
            return
        end

        player:update_housing_exterior_color(field, stain)
        printf(player, "Queued housing exterior color update: %s=%d.", field, stain)
        return
    end

    local field = mode
    local value = parse_range(args[3], nil, 0, 65535)

    if not EXTERIOR_FIELDS[field] or value == nil then
        usage(player)
        return
    end

    player:update_housing_exterior(field, value)
    printf(player, "Queued housing exterior update: %s=%d.", field, value)
end

local function handle_interior(player, args)
    local field = lower(args[2])

    if field == "preset" then
        local preset_name = lower(args[3])
        local preset = INTERIOR_PRESETS[preset_name]

        if preset == nil then
            usage(player)
            return
        end

        for _, entry in ipairs(preset) do
            player:update_housing_interior(entry[1], entry[2])
        end

        printf(player, "Queued housing interior preset: %s.", preset_name)
        return
    end

    local value = parse_non_negative_integer(args[3], 4294967295)

    if not INTERIOR_FIELDS[field] or value == nil then
        usage(player)
        return
    end

    player:update_housing_interior(field, value)
    printf(player, "Queued housing interior update: %s=%d.", field, value)
end

local function handle_preset_check(player, preset)
    if preset.kind == "latest" then
        player:check_latest_housing_preset(preset.scope)
        printf(player, "Queued ReMakePlace preset check: latest (%s).", preset.scope)
    elseif preset.kind == "repeat" then
        player:check_repeated_housing_preset()
        printf(player, "Queued ReMakePlace preset check: repeat.")
    else
        player:check_housing_preset(preset.path, preset.scope)
        printf(player, "Queued ReMakePlace preset check: %s (%s).", preset.path, preset.scope)
    end
end

local function handle_preset_apply(player, preset)
    if preset.kind == "latest" then
        player:apply_latest_housing_preset(preset.scope, preset.reload)
        printf(player, "Queued ReMakePlace housing preset: latest (%s)%s.", preset.scope, reload_suffix(preset.reload))
    elseif preset.kind == "repeat" then
        player:repeat_housing_preset(preset.reload)
        printf(player, "Queued ReMakePlace housing preset: repeat%s.", reload_suffix(preset.reload))
    else
        player:apply_housing_preset(preset.path, preset.scope, preset.reload)
        printf(player, "Queued ReMakePlace housing preset: %s (%s)%s.", preset.path, preset.scope, reload_suffix(preset.reload))
    end
end

local function handle_preset(player, args)
    local preset = parse_preset_args(args)

    if preset == nil then
        usage(player)
        return
    end

    if preset.check_only then
        handle_preset_check(player, preset)
        return
    end

    handle_preset_apply(player, preset)
end

local function handle_givekit(player, args)
    local kit = lower(args[2])

    if kit ~= "indoor" and kit ~= "outdoor" and kit ~= "npc" then
        usage(player)
        return
    end

    player:give_housing_kit(kit)
    printf(player, "Queued housing %s kit.", kit)
end

local function handle_apartment(player, args)
    local room = parse_range(args[2], 1, 1, MAX_APARTMENT_ROOM_NUMBER)

    if room == nil then
        usage(player)
        return
    end

    player:ensure_test_apartment(room)
    printf(player, "Created or refreshed your local apartment room %d.", room)
end

local COMMAND_HANDLERS = {
    apartment = handle_apartment,
    enter = handle_enter,
    exit = handle_exit,
    exterior = handle_exterior,
    givekit = handle_givekit,
    greeting = handle_greeting,
    info = handle_info,
    interior = handle_interior,
    light = handle_light,
    name = handle_name,
    preset = handle_preset,
    reload = handle_reload,
    reset = handle_reset,
    testhouse = handle_testhouse,
}

function onCommand(player, args, name)
    local subcommand = lower(args[1])

    if subcommand == nil then
        handle_testhouse(player, args)
        return
    end

    local handler = COMMAND_HANDLERS[subcommand]

    if handler == nil then
        usage(player)
        return
    end

    handler(player, args)
end
