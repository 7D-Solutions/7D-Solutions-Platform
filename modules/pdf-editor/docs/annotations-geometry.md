# Annotation Geometry Spec

Locked conventions for annotation geometry computations in the pdf-editor module.
These are the authoritative definitions — tests and renderer are derived from this document.

---

## Coordinate Conventions

| Space | Origin | Y direction |
|-------|--------|-------------|
| Screen space | top-left | downward (y increases toward bottom) |
| PDF space | bottom-left | upward (y increases toward top) |

All annotation fields (`x`, `y`, `x2`, `y2`, `leader_x`, `leader_y`) are in **screen space**.
The renderer converts to PDF space before writing path objects:

```
pdf_y = page_height - screen_y
```

---

## ARROW Geometry

### Fields

| Field | Default | Description |
|-------|---------|-------------|
| `x`, `y` | required | Tail point (screen space) |
| `x2`, `y2` | `x+50, y` | Tip point (screen space) |
| `arrowhead_size` | `10.0` | Barb length in points |
| `stroke_width` | `2.0` | Shaft and barb line width in points |
| `color` | `#FF0000` | Shaft and barb color |

### Arrowhead

Open V shape at the tip. No tail decoration.

**Locked constants:**

- Spread factor: **0.4**
- Half-angle from shaft axis: **arctan(0.4) ≈ 21.8°**
- Total opening angle: **≈ 43.6°**

**Formula** (screen space, implemented in `arrow_geometry`):

```
unit vector along shaft:
  dx = tip_x - tail_x
  dy = tip_y - tail_y
  len = sqrt(dx² + dy²)   [clamped to 0.001 to avoid div-by-zero]
  ux = dx / len
  uy = dy / len

barb endpoints:
  barb1_x = tip_x - head_size * (ux + 0.4 * uy)
  barb1_y = tip_y - head_size * (uy - 0.4 * ux)
  barb2_x = tip_x - head_size * (ux - 0.4 * uy)
  barb2_y = tip_y - head_size * (uy + 0.4 * ux)
```

**Golden values** (rightward arrow, tail=(0,0), tip=(100,0), head_size=10):

```
barb1 = (90, 4)
barb2 = (90, -4)
```

### Degenerate case

When tail == tip the shaft length is clamped to 0.001. The unit vector approaches zero
and both barbs collapse to the tip point. No panic; the two barb lines are zero-length.

---

## BUBBLE Leader-Line Geometry

Leader lines connect the bubble center to an external target point.

### Fields

| Field | Default | Description |
|-------|---------|-------------|
| `x`, `y` | required | Top-left of bubble bounding box (screen space) |
| `bubble_size` | `24.0` | Diameter in points |
| `has_leader_line` | `false` | Enable leader line |
| `leader_x`, `leader_y` | — | Target point (screen space) |
| `leader_stroke_width` | `1.5` | Line width in points |
| `leader_color` | bubble border color | Line color |

### Formula (implemented in `leader_geometry`)

The origin is the **geometric center** of the bubble regardless of shape.

```
radius = bubble_size / 2

origin_x   = anchor_x + radius           [screen → PDF: unchanged]
origin_pdf_y = page_height - anchor_y - radius

target_x   = leader_x
target_pdf_y = page_height - leader_y
```

**Golden values** (anchor=(100,200), leader=(50,300), bubble_size=24, page_height=792):

```
origin  = (112, 580)   [PDF space]
target  = (50,  492)   [PDF space]
```
