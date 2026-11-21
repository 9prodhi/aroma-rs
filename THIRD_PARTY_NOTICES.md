# Third-party notices

aroma-rs is an independent re-implementation. Its own code is licensed under
Apache-2.0, but third-party excerpts retain their source licenses.

Task files under `xtask/tasks/**` contain short `query` excerpts from pinned
public repositories. Each task identifies its repository, commit and source path;
additional Salesforce corpus licenses are recorded in
[`xtask/corpora/salesforce.toml`](xtask/corpora/salesforce.toml). The following
notices apply to excerpts from NPSP and NebulaLogger. CC0-1.0 sources, including
`trailheadapps/apex-recipes` and `trailheadapps/lwc-recipes`, require no notice.

## NPSP — BSD-3-Clause

Source: `SalesforceFoundation/NPSP`; excerpts in `xtask/tasks/**` whose repository
or component source points to NPSP.

Copyright (c) 2015, Salesforce.org
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.
3. Neither the name of the copyright holder nor the names of its contributors
   may be used to endorse or promote products derived from this software without
   specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED
AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

## NebulaLogger — MIT

Source: `jongpie/NebulaLogger`; excerpts in `xtask/tasks/**` whose repository
points to NebulaLogger.

Copyright (c) 2018 Jonathan Gillespie

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

## Aroma paper running example

`concepts/aroma-concepts/tests/concepts.rs` uses the iterator-loop running example
from S. Luan, D. Yang, C. Barnaby, K. Sen, and S. Chandra, *Aroma: Code
Recommendation via Structural Code Search*, OOPSLA 2019, arXiv:1812.01158. The
example is used as a small compatibility test; this citation is attribution, not
a claim that the paper is licensed under Apache-2.0.

The Aroma reference implementation and datasets are separately licensed
CC BY-NC-SA 4.0, fetched for testing and never vendored. See the
[project README](README.md#license-and-third-party-materials) for the reference
implementation and golden-fixture provenance.
