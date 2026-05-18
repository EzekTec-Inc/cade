CADE_UI = {}
setmetatable(CADE_UI, {
    __index = function(t, key)
        if key == "sidebar" then
            return "from_metamethod"
        end
    end
})
