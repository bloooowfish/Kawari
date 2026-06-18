-- Generic warp shared by all personal house entrances

-- Scenes
SCENE_ENTER_PROMPT = 00000 -- "Enter the estate hall?" prompt

function onTalk(target, player)
    player:play_scene(SCENE_ENTER_PROMPT, HIDE_HOTBAR, {})
end

function onReturn(scene, results, player)
    local ENTER_HOUSE <const> = 1

    player:finish_event()

    if results[1] == ENTER_HOUSE then
        player:enter_local_house()
    end
end
