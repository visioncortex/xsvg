# eXtensible SVG

## Vision

To be an expressive and powerful xml based interchange format that can compile down to svg.

## Current Limitations

1. Typography, typesetting sucks
    + CSS / HTML is much better but hard to intermix

2. lack of non-affine transformations

3. gradient and blur primitive hard to work with

## Ideas

1. first class typography
    + paragraph flow, fit text within polygon
    + capitalize first letter of paragraph
    + line spacing
    + character width scaling
    + can apply fill and stroke effects on a subset of paragraph text

2. non-affine and non-destructive geometry transform pipeline

3. gradient mesh with cracks and T-junctions
    + transparency capable (feathering / fade effect)

## Constraints

A "compiler" or "rendering engine" that can output low-level svg, with various quality level of approximation.
Should be compilable to wasm (rust) and runs in browser on client side.

## Nice to have, optional

A web-gpu based reference renderer. Note that we only compile down to a subset of svg, so that we don't have to achieve full svg test suite coverage, just impl what we use.