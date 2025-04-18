# 0.2.0 (Unreleased)

- Clear the output directory as the first step of `hinoki build`
- Change conditionally-defined template variables to always-defined (absence of
  the value should be checked with `is none` instead of `is undefined` now)
- Add `--port` and `--open` flags to `hinoki serve`
