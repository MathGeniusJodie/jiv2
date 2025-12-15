# Jodie's Image Viewer 2 (jiv2)

Having fun, please use chafa instead of this project

## Usage

```bash
# best looking method but bad compatibility
sextant path/to/image.jpg [--width <width>] [--edges]
# best resolution method but dotted look
braille path/to/image.jpg [--width <width>] [--edges]
# best compatibility method but worst resolution
quadrant path/to/image.jpg [--width <width>] [--edges]
# other braille method, will replace with actal octants when unicode support is better
octant path/to/image.jpg [--width <width>] [--edges]
```

- the edges flag enhances edges
- the width flag sets the width of the image in characters (default is terminal width)