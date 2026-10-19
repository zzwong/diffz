"""Local development codegen switch."""

def _bool_impl(_ctx):
    return []

bool_flag = rule(implementation = _bool_impl, build_setting = config.bool(flag = True))
