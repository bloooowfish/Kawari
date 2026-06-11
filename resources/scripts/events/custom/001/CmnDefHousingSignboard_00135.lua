-- Placard, in front of housing plots in residential wards

-- TODO: Figure out how to make the placard menu actually show, might be related to a packet sent just before playing the scene. But when we do figure it out, we should display that all plots are not available for purchase until housing is actually implemented.

--Scenes
SCENE_00000 = 00000 -- Displays placard menu, but does nothing on Kawari for now
SCENE_00001 = 00001 -- "You have submitted a lottery result for plot {param 1}, ward {param 0}, <Housing ward name>. Your lottery number is {param 2}, Results will be available from {housing auction datetime, not controlled by scene params}."
SCENE_00002 = 00002 -- "Unable to claim refund. You cannot carry any more gil."

function onTalk(target, player, event_arg)
    local context = player:get_housing_ward_context()
    local placard = player:get_housing_placard_location(event_arg or 0)

    player:show_housing_placard(context.ward_index, placard.division, placard.plot_index)
    player:play_scene(SCENE_00000, HIDE_HOTBAR, {})
end

function onReturn(scene, results, player)
    player:finish_event()
end
