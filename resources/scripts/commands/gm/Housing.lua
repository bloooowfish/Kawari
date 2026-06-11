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
    capture_shirogane_medium_mist_style = {
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

INTERIOR_PRESETS.capture_shirogane_medium = INTERIOR_PRESETS.capture_shirogane_medium_mist_style
INTERIOR_PRESETS.retail_shirogane_medium = INTERIOR_PRESETS.capture_shirogane_medium_mist_style
INTERIOR_PRESETS.capture = INTERIOR_PRESETS.capture_shirogane_medium_mist_style

local function lower(value)
    if value == nil then
        return nil
    end

    return string.lower(value)
end

local function usage(player)
    printf(player, "Usage: !housing testhouse [personal|fc] [small|medium|large] [territory_id] [ward] [plot]")
    printf(player, "       !housing apartment [room]")
    printf(player, "       !housing enter [apartment [room]]|exit|info")
    printf(player, "       !housing reset furniture|estate|all")
    printf(player, "       !housing light <0-5>")
    printf(player, "       !housing greeting <text>")
    printf(player, "       !housing name <text>")
    printf(player, "       !housing exterior <field> <value>")
    printf(player, "       !housing exterior color <field> <stain>")
    printf(player, "       !housing interior <field> <value>")
    printf(player, "       !housing interior preset capture_shirogane_medium_mist_style")
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

function onCommand(player, args, name)
    local subcommand = lower(args[1])

    if subcommand == nil or subcommand == "testhouse" then
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
        return
    end

    if subcommand == "enter" then
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
        return
    end

    if subcommand == "exit" then
        player:exit_test_house()
        printf(player, "Exiting your local test estate.")
        return
    end

    if subcommand == "info" then
        local context = player:get_housing_ward_context()
        printf(player, "Housing context: territory %d ward %d division %d.", context.territory_type_id, context.ward_index + 1, context.division)
        return
    end

    if subcommand == "reset" then
        local mode = lower(args[2])

        if mode ~= "furniture" and mode ~= "estate" and mode ~= "all" then
            usage(player)
            return
        end

        player:reset_housing(mode)
        printf(player, "Queued housing reset: %s.", mode)
        return
    end

    if subcommand == "light" then
        local level = parse_range(args[2], nil, 0, 5)

        if level == nil then
            usage(player)
            return
        end

        player:update_housing_light(level)
        printf(player, "Queued housing light level update: %d.", level)
        return
    end

    if subcommand == "greeting" then
        local greeting = join_args(args, 2)

        if greeting == "" then
            usage(player)
            return
        end

        player:update_housing_greeting(greeting)
        printf(player, "Queued housing greeting update.")
        return
    end

    if subcommand == "name" then
        local estate_name = join_args(args, 2)

        if estate_name == "" then
            usage(player)
            return
        end

        player:update_housing_name(estate_name)
        printf(player, "Queued housing name update.")
        return
    end

    if subcommand == "exterior" then
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
        return
    end

    if subcommand == "interior" then
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
        return
    end

    if subcommand == "givekit" then
        local kit = lower(args[2])

        if kit ~= "indoor" and kit ~= "outdoor" and kit ~= "npc" then
            usage(player)
            return
        end

        player:give_housing_kit(kit)
        printf(player, "Queued housing %s kit.", kit)
        return
    end

    if subcommand == "apartment" then
        local room = parse_range(args[2], 1, 1, MAX_APARTMENT_ROOM_NUMBER)

        if room == nil then
            usage(player)
            return
        end

        player:ensure_test_apartment(room)
        printf(player, "Created or refreshed your local apartment room %d.", room)
        return
    end

    usage(player)
end
