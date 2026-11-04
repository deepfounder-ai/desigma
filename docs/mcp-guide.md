# Building designs with Desigma MCP

A short guide for agents. The loop is: **read assets, build, look, fix.**

1. **Read.** `desigma_get_assets` lists components, styles and variables. `desigma_get_tree {node_id, depth}` shows the layers that already exist. Reuse what is there; do not paint loose colours and numbers when a variable or component exists.
2. **Prepare.** Missing tokens: `desigma_set_variables` (by name, repeatable). A missing building block: build it once with `desigma_apply_nodes`, then `desigma_create_component {node_id}`.
3. **Build.** One `desigma_apply_nodes` call per screen or section. Use `instance_of` for components (with `overrides` by layer name) and `bind` for variables (by name). Use `width`/`height` of `"hug"` and `"fill"` with `auto_layout` instead of computing positions. Add `validate_only: true` to check a big tree first. Each call is one undo step and changes nothing if any part is invalid.
4. **Look.** `desigma_screenshot {node_id, scale}` returns a PNG path; open it and compare with what was asked. `desigma_get_tree` tells you the numbers behind what you see.
5. **Fix.** Update with `node_id` (only the fields you pass change). To rebuild a list use `"replace_children": true`. To change text, colour or visibility inside an instance use `desigma_override_instance`. If a change is wrong, `desigma_undo`.

## Complete example: a login card

```jsonc
// 1. tokens
desigma_set_variables
{"collections":[{"name":"Brand","modes":["Light","Dark"],"variables":[
  {"name":"Color/Primary","type":"color","values":{"Light":"#3366ff","Dark":"#88aaff"}},
  {"name":"Color/Surface","type":"color","values":{"Light":"#ffffff","Dark":"#1c1c1e"}},
  {"name":"Space/M","type":"number","value":16},
  {"name":"Radius/L","type":"number","value":16}]}]}

// 2. a button, built once, then made a component (returns node ids; here the page is 1)
desigma_apply_nodes
{"parent_id":1,"nodes":[{"kind":"frame","name":"Button","width":"hug","height":"hug","corner_radius":10,"fill":"#3366ff",
  "bind":{"fill":"Color/Primary"},
  "auto_layout":{"direction":"horizontal","padding":[12,20],"counter_align":"center"},
  "children":[{"kind":"text","name":"Label","text":"Button","text_style":{"font_size":15,"font_weight":600,"color":"#ffffff"}}]}]}
desigma_create_component {"node_id": <Button id>}

// 3. the screen: a card with a title, a field and two instances of the button
desigma_apply_nodes
{"parent_id":1,"nodes":[{"kind":"frame","name":"Login card","x":80,"y":80,"width":360,"height":"hug",
  "fill":"#ffffff","corner_radius":16,"effects":[{"type":"drop_shadow","color":"#00000026","y":8,"blur":24}],
  "auto_layout":{"direction":"vertical","gap":16,"padding":24},
  "bind":{"fill":"Color/Surface","gap":"Space/M","radius":"Radius/L"},
  "children":[
    {"kind":"text","name":"Title","text":"Welcome back","width":"fill","text_style":{"font_size":24,"font_weight":700}},
    {"kind":"rectangle","name":"Email field","width":"fill","height":44,"corner_radius":8,"fill":"#f2f2f7","stroke":{"color":"#d1d1d6","width":1}},
    {"instance_of":<Button id>,"name":"Sign in","width":"fill","overrides":{"Label":{"text":"Sign in"}}},
    {"instance_of":<Button id>,"name":"Create account","width":"fill","overrides":{"Label":{"text":"Create account"}}}]}]}

// 4. look
desigma_screenshot {"node_id": <Login card id>, "scale": 2}

// 5. fix: drop the second button and add a link instead (one undo step)
desigma_apply_nodes
{"nodes":[{"node_id":<Login card id>,"replace_children":true,"children":[
  {"node_id":<Title id>},{"node_id":<Email field id>},{"node_id":<Sign in id>},
  {"kind":"text","name":"Forgot","text":"Forgot password?","text_style":{"font_size":13,"color":"#3366ff"}}]}]}
```

## Rules that save round trips

- Ids come back in spec order in `nodes[].node_id`; keep them instead of searching again.
- Names inside `overrides` are layer names of the component, first match depth first. A wrong name lists the real ones.
- Gap and padding bindings need `auto_layout` in the same spec; fill and stroke bindings need a fill or stroke.
- Errors name the spec path (`nodes[0].children[2]`), so fix that spec and repeat the whole call.
- Edits made in the editor and by the agent share one undo stack. Never undo more steps than you made.
