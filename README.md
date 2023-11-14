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

```toml
# The defaults table lets you set defaults for frontmatter fields for multiple
# files at once (matched via glob syntax).

# A lot of things that other static site generators do by default with no
# opt-out are handled via defaults in hinoki, such as transforming markdown to
# HTML.
[defaults."*.md"]
process_content = "markdown_to_html"

[defaults."blog/*"]
# It is also possible to use basic templating for paths and titles.
# This uses a MiniJinja engine with `{{` and `}}` shortened to `{` and `}`.
#
# For now, only the `slug` variable is passed, but something like the below
# will be supported in the future.
path = "blog/{date.year}/{date.month}/{date.day}/{slug}/"
template = "blog_article.html"
```

### Frontmatter

If the first line of a file in `content` is found to be `+++`, the contents of the following line up until the second `+++` line are read as frontmatter.
Just like the configuration file, this frontmatter is written in TOML.
The following things can be configured in frontmatter (and thus config defaults):

```toml
# Set this page to be a draft.
#
# This excludes it from build output by default.
# Use the `--drafts` command-line option to include draft pages.
draft = true
# Path of the teamplate to use for this page.
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
path = "blog/{slug}.html"
# Page title.
title = "Foo"
# Page date.
date = 2023-03-03
# Custom slug for this page.
slug = "bar"
```
