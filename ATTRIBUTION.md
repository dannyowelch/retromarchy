# System catalog attribution

`resources/systems.json` is derived from EmulationStation Desktop Edition's Linux systems list.

- Source: https://gitlab.com/es-de/emulationstation-de
- File: `resources/systems/linux/es_systems.xml`
- Pinned tag: v3.1.1 (commit `a59b8016be3ccaab0a678a552128d06b32e7dc01`)
- Derived fields only: `<name>` → `folder_id`, `<fullname>` → `display_name`, `<extension>` normalized to lowercase without a leading dot
- Launch commands from that file are not included

ES-DE is MIT licensed. Copyright (c) 2020-2024 Leon Styhre and contributors.

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
