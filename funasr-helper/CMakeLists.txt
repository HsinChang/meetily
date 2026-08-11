# funasr-helper: persistent Fun-ASR-Nano transcription sidecar for Meetily.
#
# Unlike llama-helper (Rust, llama-cpp-2 bindings), this must be C++: the Fun-ASR
# SAN-M audio encoder is a hand-built ggml graph with no Rust binding. It fetches the
# same pinned llama.cpp that upstream Fun-ASR pins, so encoder and LLM agree on ggml.
#
#   cmake -B build -DCMAKE_BUILD_TYPE=Release
#   cmake --build build -j            # -> build/bin/funasr-helper
#
# Reuse an existing llama.cpp checkout instead of fetching (much faster rebuilds):
#   -DFETCHCONTENT_SOURCE_DIR_LLAMA=/path/to/llama.cpp
cmake_minimum_required(VERSION 3.16)

# Match the deployment target the app is built with (see
# frontend/src-tauri/.cargo/config.toml). This build runs outside cargo, so it does not
# inherit that config — and without an explicit value CMake targets the *host* OS. That
# shipped a sidecar stamped `minos 26.0` inside an app targeting 14.2, which dyld refuses
# to load on anything older, silently breaking Fun-ASR for all but the newest macOS.
# Must be set before project().
if(APPLE AND NOT CMAKE_OSX_DEPLOYMENT_TARGET)
    set(CMAKE_OSX_DEPLOYMENT_TARGET "14.2" CACHE STRING "Minimum macOS version" FORCE)
endif()

project(funasr-helper CXX C)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)   # static deps -> self-contained sidecar
set(CMAKE_RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)
if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
    set(CMAKE_BUILD_TYPE Release CACHE STRING "" FORCE)
endif()

include(FetchContent)
set(LLAMA_BUILD_TESTS    OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_TOOLS    OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_SERVER   OFF CACHE BOOL "" FORCE)
set(LLAMA_CURL           OFF CACHE BOOL "" FORCE)

# Pinned to the same commit upstream Fun-ASR pins (runtime/llama.cpp/CMakeLists.txt).
# Bumping this must be done in lockstep with re-validating the encoder graph.
FetchContent_Declare(llama
    GIT_REPOSITORY https://github.com/ggml-org/llama.cpp.git
    GIT_TAG        8086439a4cea94c71a5dfb8fe4ad1546aebd640f)
FetchContent_MakeAvailable(llama)

find_package(Threads REQUIRED)

add_executable(funasr-helper src/funasr-helper.cpp)
target_link_libraries(funasr-helper PRIVATE llama ggml Threads::Threads)
install(TARGETS funasr-helper RUNTIME DESTINATION bin)
