# Hinoki

A simple, yet very flexible static site generator. It's also super fast.

It's early days for this project, and this readme is currently all there is in terms of documentation.
Feel free to open issues for anything that is unclear!

## Features

- [x] Templating (with [MiniJinja](https://docs.rs/minijinja/latest/minijinja/))
  - [ ] Use of template expressions in `content` files
- [x] Markdown to HTML conversion
  - [x] Syntax highlighting
  - [x] Footnotes
  - [ ] Other markdown features like tables
- [x] TOML frontmatter
- [x] Custom defaults for frontmatter via path patterns
- [ ] Pagination
- [ ] Development server
- [ ] SCSS compilation
- [ ] Page summaries

## How to install

At this time, no binaries are being distributed yet.

Since `hinoki` is written in Rust, you need a Rust toolchain to build it yourself.
You can use `cargo install --git https://github.com/jplatte/hinoki` to build and install it to `~/.cargo/bin/`.

## How to use

The basic structure for a site you build with `hinoki` is this:

```sh
├ config.toml  # Basic configuration
├ build        # Default output directory
├ content      # Content of the site, e.g. markdown files
└ theme
  ├ static     # Static files
  ├ sublime    # Sublime `.tmTheme` files
  └ templates  # MiniJinja templates
```

After installing it, you can run `hinoki build` in such a directory to populate the output directory.

### Configuration

Lots of things that you would configure globally with other static site generators are configured on a per-file basis with hinoki.
To make this ergonomic, you can apply any file settings to many files at once using `config.toml`'s `files` section.
There are also a few other things you can set in this file, as shown in the example below:

```toml
# The output directory.
#
# Default value: `build`
output_dir = "public"

# Convert all .md files to HTML.
[files."*.md"]
process_content = "markdown_to_html"

[files."blog/*"]
template = "blog_article.html"
# It is also possible to use basic templating for paths and titles.
# This uses a MiniJinja engine with `{{` and `}}` shortened to `{` and `}`.
date = "{source_file_stem|date_prefix}"
slug = "{source_file_stem|strip_date_prefix}"
# The path template will be expanded last (regardless of its position in the
# frontmatter / defaults) and can use `slug`, `date` and `title`, possibly more
# other fields in the future.
path = "/{date|dateformat(format='[year]/[month]')}/{slug}/index.html"

# This section is for arbitrary user-defined data.
#
# Unknown fields outside of this table will cause errors, so typos get caught.
[extra]
# Available as `config.extra.author` in template code.
author = "Erika Mustermann"
```

### Frontmatter

Since some configuration options make a lot more sense to be specified on individual files
and one global config file doesn't scale very well,
you can also place configuration in content files themselves.

This is done using TOML "frontmatter", that is an embedded TOML document at the start of the file.
It is introduced by starting the file with a line that contains exactly `+++`,
followed by the TOML document which is then terminated with another `+++` line.

Here is the full set of things you can currently configure through frontmatter (or `config.toml`):

```toml
# Set this page to be a draft.
#
# This excludes it from build output by default.
# Use the `--drafts` command-line option to include draft pages.
draft = true
# Path of the template to use for this page.
#
# Relative to the `theme/templates` directory.
template = "my_tpl.html"
# What kind of processing should be done on the content, if any.
#
# For now, only markdown to HTML conversion is available.
process_content = "markdown_to_html"
# Syntax highlighting theme for markdown code blocks.
#
# If there is only one `.tmTheme` file in `theme/sublime`, it will be used by
# default, i.e. there is no need to specify this field anywhere.
syntax_highlight_theme = "visual-studio-dark"
# Custom rendered path for this page.
path = "/blog/{slug}.html"
# Page title.
title = "Foo"
# Page date.
date = 2023-03-03
# Custom slug for this page.
slug = "bar"
```
