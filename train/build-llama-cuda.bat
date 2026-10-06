@echo off
rem A CUDA build of llama-server for the 3090 (sm_86) only, beside the CPU build in C:\llama-cpp\build.
rem NMake with nvcc named outright: CUDA 11.7 installed no build extensions into this Visual Studio.
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
rem NMake (in MSVC) builds on one core: light while the owner games.
cmake --fresh -G "NMake Makefiles" -S C:\llama-cpp -B C:\llama-cpp\build-cuda -DCMAKE_BUILD_TYPE=Release -DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES=86 "-DCMAKE_CUDA_COMPILER=C:/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v11.7/bin/nvcc.exe" "-DCMAKE_CUDA_FLAGS=-allow-unsupported-compiler -D_ALLOW_COMPILER_AND_STL_VERSION_MISMATCH" -DLLAMA_CURL=OFF -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF
if errorlevel 1 exit /b 1
cmake --build C:\llama-cpp\build-cuda -j 2 --target llama-server
