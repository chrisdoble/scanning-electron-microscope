# %%
from build123d import *
from ocp_vscode import show

# %%
with BuildPart() as box_builder:
    Box(1, 1, 1)

# show(box_builder)

# %%
export_stl(box_builder.part, "case.stl")
