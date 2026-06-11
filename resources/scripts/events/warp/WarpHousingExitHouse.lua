-- Generic warp shared by all personal houses

-- Scenes
SCENE_EXIT_PROMPT = 00000 -- "Leave the estate hall?" prompt

function onTalk(target, player)
    player:play_scene(SCENE_EXIT_PROMPT, HIDE_HOTBAR, {})
end

function onReturn(scene, results, player)
    local LEAVE_HOUSE <const> = 1
    if results[1] == LEAVE_HOUSE then
        player:finish_event()
        player:exit_test_house()
        return
    end

    player:finish_event()
end
