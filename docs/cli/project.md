# `ove project`

```sh
ove project list [--search <text>]
ove project create <name> [--aspect 16:9|9:16|1:1|4:5|4:3|3:4] [--open]
ove project show <project>
ove project rename <project> <name>
ove project duplicate <project> [--name <name>]
ove project remove <project> --yes
ove project open <project>
ove project reveal <project>
```

`<project>` is a name (exact, then case-insensitive), an id or a bundle path. New projects start with the tracks
Text, Captions, Overlay, Main and Audio. `remove` moves the bundle to the Trash and is refused while the project is
open anywhere. `open` needs the running app. Renames are revisions; the bundle folder keeps its name.

Examples (exit codes as documented):

```sh
ove project create Promo --aspect 9:16 # exit 0
ove project list --json # exit 0
ove project show Promo # exit 0
ove project rename Promo PromoV2 # exit 0
ove project duplicate PromoV2 --name Draft # exit 0
ove project remove Draft # exit 1
ove project create Bad --aspect 2:1 # exit 1
```
