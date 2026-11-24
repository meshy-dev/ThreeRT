# Third-party notices

ThreeRT is MIT-licensed (LICENSE). Parts of it follow formulas or reference code published under licences that ask for
a notice. They are listed here with where they are used.

## Surface BSDF and spot lights (`renderer/shaders/pathtrace.wgsl`, `renderer/shaders/live.wgsl`, `renderer/src/lib.rs`, `renderer/src/scene.rs`)

### Filament (documentation: "Physically Based Rendering in Filament")

`vis_smith_joint_approx` (pathtrace.wgsl), `vis_smith` (live.wgsl) and the same expression in lib.rs
`specular_albedo` evaluate the square-root-free approximation of the height-correlated Smith-GGX visibility in the form
of Filament's `V_SmithGGXCorrelated_Fast` (0.5 / mix(2 n.l n.v, n.l + n.v, alpha); it credits Hammon, "PBR Diffuse
Lighting for GGX+Smith Microsurfaces", GDC 2017). The multiple-scattering scale `1 + f0 (1 / E - 1)` and the grazing reflectance
`saturate(50 x mean(f0))` appear there in the same form (after Turquin 2019 and Lagarde & de Rousiers 2014).

    Filament, https://github.com/google/filament (shaders/src/surface_brdf.fs; documentation:
    https://google.github.io/filament/Filament.md.html)
    Copyright 2023 The Android Open Source Project
    Licensed under the Apache License, Version 2.0 (the "License"); you may not use this file except in compliance
    with the License. You may obtain a copy of the License at http://www.apache.org/licenses/LICENSE-2.0
    Unless required by applicable law or agreed to in writing, software distributed under the License is distributed
    on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the License for
    the specific language governing permissions and limitations under the License.

### glTF 2.0, KHR_lights_punctual (The Khronos Group)

The spot cone's angular attenuation (pathtrace.wgsl `local_light`, scene.rs `spot_cone`) is the extension's reference
attenuation (scale 1 / max(0.001, cos(inner) - cos(outer)), linear in the cosine, squared), which is `getAngleAtt` of
Lagarde & de Rousiers, "Moving Frostbite to Physically Based Rendering" (SIGGRAPH 2014 course notes), Listing 4.

    Copyright 2014-2021 The Khronos Group Inc. SPDX-License-Identifier: CC-BY-4.0
    https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_lights_punctual

### three.js

`SpotLight.penumbra` has three.js's meaning (the inner cone is `angle x (1 - penumbra)`, three.js `WebGLLights`); no
three.js code is used for the falloff.

    The MIT License. Copyright (c) 2010-2026 three.js authors. https://github.com/mrdoob/three.js

## Sky (`renderer/src/sky.rs`, `tools/three/sky.ts`)

### Precomputed Atmospheric Scattering (Eric Bruneton, 2017)

The constants of Earth's atmosphere in `renderer/src/sky.rs` and `tools/three/sky.ts` (the Rayleigh scattering
coefficient 1.24062e-6 λ⁻⁴ per metre at 680 / 550 / 440 nm, the scale heights 8000 m and 1200 m, the Mie extinction
5.328e-3 / 1200 m per metre with a single-scattering albedo of 0.9, the ozone layer's profile and its absorption for
300 Dobson units) are those of that project's demo (`atmosphere/demo/demo.cc`,
https://ebruneton.github.io/precomputed_atmospheric_scattering/). No code of it is used. Its licence (BSD 3-Clause):

```
Copyright (c) 2017 Eric Bruneton
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:
1. Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the distribution.
3. Neither the name of the copyright holders nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF
THE POSSIBILITY OF SUCH DAMAGE.
```

## Post chain (`renderer/src/post.rs`)

### Academy Color Encoding System (ACES) reference transforms

`renderer/src/post.rs` (colour grading and the tone curve, `Tonemapper`) follows the structure of the Academy's
reference rendering and takes from the ACES 1.3 reference transforms (https://github.com/ampas/aces-dev, tag v1.3:
`RRT.ctl`, `ACESlib.RRT_Common.ctl`, `ACESlib.ODT_Common.ctl`, `ACESlib.Utilities_Color.ctl`,
`LMT.Academy.BlueLightArtifactFix.ctl`): the constants of the "glow" and "red modifier", the desaturation factors,
the AP0 / AP1 chromaticities, the blue-light-artifact-fix matrix, and the functions `glow_fwd`, `sigmoid_shaper` and
`cubic_basis_shaper` re-expressed in Rust. The tone curve itself is not the Academy's.

> License Terms for Academy Color Encoding System Components
>
> Academy Color Encoding System (ACES) software and tools are provided by the Academy under the following terms and
> conditions: A worldwide, royalty-free, non-exclusive right to copy, modify, create derivatives, and use, in source and
> binary forms, is hereby granted, subject to acceptance of this license.
>
> Copyright © 2015 Academy of Motion Picture Arts and Sciences (A.M.P.A.S.). Portions contributed by others as
> indicated. All rights reserved.
>
> Performance of any of the aforementioned acts indicates acceptance to be bound by the following terms and conditions:
>
> * Copies of source code, in whole or in part, must retain the above copyright notice, this list of conditions and the
>   Disclaimer of Warranty.
> * Use in binary form must retain the above copyright notice, this list of conditions and the Disclaimer of Warranty
>   in the documentation and/or other materials provided with the distribution.
> * Nothing in this license shall be deemed to grant any rights to trademarks, copyrights, patents, trade secrets or
>   any other intellectual property of A.M.P.A.S. or any contributors, except as expressly stated herein.
> * Neither the name "A.M.P.A.S." nor the name of any other contributors to this software may be used to endorse or
>   promote products derivative of or based on this software without express prior written permission of A.M.P.A.S.
>   or the contributors, as appropriate.
>
> This license shall be construed pursuant to the laws of the State of California, and any disputes related thereto
> shall be subject to the jurisdiction of the courts therein.
>
> Disclaimer of Warranty: THIS SOFTWARE IS PROVIDED BY A.M.P.A.S. AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED
> WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
> PURPOSE, AND NON-INFRINGEMENT ARE DISCLAIMED. IN NO EVENT SHALL A.M.P.A.S., OR ANY CONTRIBUTORS OR DISTRIBUTORS, BE
> LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, RESITUTIONARY, OR CONSEQUENTIAL DAMAGES (INCLUDING,
> BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
> INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
> NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF
> SUCH DAMAGE.
>
> WITHOUT LIMITING THE GENERALITY OF THE FOREGOING, THE ACADEMY SPECIFICALLY DISCLAIMS ANY REPRESENTATIONS OR WARRANTIES
> WHATSOEVER RELATED TO PATENT OR OTHER INTELLECTUAL PROPERTY RIGHTS IN THE ACADEMY COLOR ENCODING SYSTEM, OR
> APPLICATIONS THEREOF, HELD BY PARTIES OTHER THAN A.M.P.A.S.,WHETHER DISCLOSED OR UNDISCLOSED.
