# Desigma Components

The component model shared by the editor and the MCP server: main components, their variants, and the properties instances can set.

## Language

**Component**:
A main layer that instances link to and synchronize from.
_Avoid_: master, symbol, source component

**Component set**:
A container whose children are the variants of one component family; it owns the family's variant properties.
_Avoid_: variant group, component group

**Variant**:
A main component inside a component set whose name encodes its variant property values, such as `Size=Large`.
_Avoid_: variant component, state

**Variant property**:
A component property whose options are the values used by the variants in a set; selecting an option chooses the matching variant.
_Avoid_: variant dimension, variant key

**Component property**:
A named, typed input on a component or component set that an instance can set: boolean, text, variant, or instance swap.
_Avoid_: prop, parameter

**Instance**:
A linked copy of a component whose subtree mirrors the main component and keeps its own overrides.
_Avoid_: copy, clone

**Override**:
A field on a layer inside an instance that intentionally differs from the main component and survives synchronization.
_Avoid_: local change, exception

**Binding**:
A link from a field on a main-component layer to a component property.
_Avoid_: connection, mapping

**Synchronization**:
Reapplying a main component to its instances while preserving their overrides.
_Avoid_: refresh, update
