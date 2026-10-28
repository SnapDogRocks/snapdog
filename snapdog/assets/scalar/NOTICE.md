# Vendored: Scalar API Reference (browser bundle)

`standalone-1.74.0.js.gz` is the file `dist/browser/standalone.js` of the npm package
[`@scalar/api-reference`](https://www.npmjs.com/package/@scalar/api-reference) version **1.74.0**
(published 2026-10-12), compressed with `gzip -9 -n`. SnapDog serves it at `/docs/assets/scalar.js`
for the optional API console (`[http].api_console = true`, builds with the `api-docs` feature), so
the console page contacts no other host for its script.

| | |
|---|---|
| Source | `https://cdn.jsdelivr.net/npm/@scalar/api-reference@1.74.0/dist/browser/standalone.js` |
| Upstream | https://github.com/scalar/scalar (`packages/api-reference`) |
| SHA-256 of the original `standalone.js` | `ea680608a3edc8fc3a27d6259d99ae8884d7061ee0149b166c01b45edb17bb73` |
| Size of the original / of the embedded `.gz` | 4,571,209 bytes / 1,318,345 bytes |
| SHA-256 of the embedded `.gz` | `a7222f8c72a0ec4eed38cafff7120449a060fce83103c6c0936fd2fefe56afb5` |

`snapdog/tests/api_console.rs` pins both hashes. To update, run
`scripts/vendor-scalar.sh <version>` and follow what it prints.

## Licence

The package is licensed under the MIT License. It is a bundle, so it also contains code of its
dependencies. Its direct dependencies are licensed MIT (all `@scalar/*` packages, `vue`, `nanoid`,
`microdiff`, `@unhead/vue`, `@vueuse/core`, `@headlessui/vue`), ISC (`yaml`) and Apache-2.0
(`fuse.js`); the bundle carries tailwindcss (MIT). Transitive dependencies were not enumerated.
All of these licences are compatible with this project's GPL-3.0-only.

```
MIT License

Copyright (c) 2023-present Scalar

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
