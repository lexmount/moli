## Typography is the new black

High quality typography, once the domain of static layout generators such as TeX
and derivatives, has commanded increasing significance in the last decade among
real time text engines and as such has received long needed resources and
attention from specification groups, web browser developers and platform
vendors. These efforts have reinvigorated the landscape, producing more
sophisticated text APIs, new OpenType specifications, increased investment
in font production technologies and a wide corpus of beautiful, comprehensive fonts
supporting modern features. Perhaps more importantly, there has been a strong
focus on extending global access to computing through both more robust font
fallback systems and Google's ever-expanding collection of Noto fonts supporting
world-wide scripts and languages.

## Portable, safe rich text layout

While modern text layout engines have grown quite sophisticated, they are unfortunately
limited to platform specific applications, requiring cross-platform code (or even
cross-platform toolkits) to specialize their text rendering implementations for
each supported operating system. An increasingly popular option to side-step this problem
is to use an abstraction wrapping an embedded web browser such as Electron. This
has advantages if your application is already using a wide range of web technologies,
but becomes a large, complicated, overbearing dependency otherwise.

In addition, while platform specific text engines and web browsers consist of high quality,
well-tested code produced by incredibly talented developers, they are all ultimately large
and complex codebases implemented in C++ which is a fundamentally unsafe language. The Rust
language has proven to be a high performance, safe alternative to C++.

Given the platform specific nature of current text engines, and the unsafe implementation
language, the obvious course of action is to build a new, cross-platform, open source text
layout engine in Rust.

## Design overview

The Rust ecosystem comprises a large collection of high quality crates, specifically in
the domains of networking and servers. The graphics situation is fairly nascent which
leads to challenges in bootstrapping projects, but also opportunities to lay a robust
foundation for future work. The Druid project is a particularly interesting offering for
UI development and its underlying abstraction for 2D graphics, piet, is a very promising
API that is a leading contender for coalescing development efforts in the graphics
domain. As the author of swash, a new crate providing font parsing and complex text shaping,
my goal is to bridge the two, building a text layout engine that is based on the swash
primitives and exposes the piet text API.

Rich text layout is a complex process involving cross-cutting concerns across a wide
swath of both the Unicode and OpenType specifications interleaved with design intent,
available resources, internationalization considerations, and performance. Broadly, the
process can be represented by a pipeline consisting of the following phases:

* Bidirectional processing: identifies spans of text containing runs of mixed directionality
and assigns specific "levels" according the Unicode Bidirectional Algorithm (UAX #9).
These levels are used to determine the directionality of text and to ultimately reorder
runs later in the pipeline.

* Itemization: splits a paragraph into runs that are appropriate for shaping. This takes
Unicode scripts, locales, BiDi levels, fonts and other text attributes into account.

* Font selection/fallback: selects an appropriate font for each run based on desired
font attributes, script, locale and character coverage. Ideally, this will select the
font specified by the user, but will prioritize selecting for appropriate coverage and
readable text (no tofu). This stage can break runs produced by itemization if necessary.

* Shaping: converts each run of characters into a sequence of positioned glyphs using
the selected font and based on writing system rules and selected features. This will be
provided by the swash crate.

* Line breaking: breaks runs to fit within some desired maximum extent based on the
Unicode Line Breaking Algorithm (UAX #14). Reorders glyph runs per-line according to the
previously computed BiDi levels. Computes final layout of lines and glyphs.

Ultimately, the purpose of this project is to take the input specified by the piet text
API and apply the above pipeline, producing a final paragraph layout suitable for
hit-testing and rendering. Appropriate data structures and algorithms for implementing
such will be chosen and documented as development proceeds.

## Extensions to the piet text API

The piet text API currently provides a fairly minimal set of attributes to provide text
rendering across the range of supported backends. This project initially proposes three
specific additions to the API.

With respect to text attributes, there is strong desire to offer both OpenType feature
selectors and variation axes settings. The specific design of such to be discussed and agreed
upon with the piet maintainer.

To provide proper hit-testing and selection of bidirectional text, the API will also
require support for some form of affinity with regard to the bounds of a selection.
This might require a more sophisticated selection API beyond the currently supported
Unicode scalar offsets.

If time permits, additional features might include user specified letter, word, and
line spacing and customizable decorations such as custom offsets, sizes, and
colors for underline and strikeout strokes.

## Further considerations

There exist some nice-to-have layout features that are out of scope for the current development
cycle of this project-- specifically, more sophisticated segmentation using a
dictionary based approach for languages such as Thai and shaper/dictionary driven
hyphenation and justification.

Additionally, font selection and fallback for this project will be provided by a related
library that provides a pre-baked collection of well-known platform specific fonts with
appropriate script coverage. This is a fairly simple and robust solution, but there are
opportunities for future work in more efficient integration with platform font libraries
without taking on large dependencies. This would require cross-vendor coordination and is
also out of scope for this project.
