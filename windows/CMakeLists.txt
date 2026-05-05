# Project-level configuration.
cmake_minimum_required(VERSION 3.14)
project(gst_audio_flutter LANGUAGES CXX)

# The name of the executable created for the application. Change this to change
# the on-disk name of your application.
set(BINARY_NAME "gst_audio_flutter")

# Explicitly opt in to modern CMake behaviors to avoid warnings with recent
# versions of CMake.
cmake_policy(VERSION 3.14...3.25)

set(GSTREAMER_ROOT_WINDOWS "" CACHE PATH "Path to the GStreamer MSVC x86_64 SDK root")
if(NOT GSTREAMER_ROOT_WINDOWS AND DEFINED ENV{GSTREAMER_ROOT_WINDOWS})
  set(GSTREAMER_ROOT_WINDOWS "$ENV{GSTREAMER_ROOT_WINDOWS}" CACHE PATH "Path to the GStreamer MSVC x86_64 SDK root" FORCE)
elseif(NOT GSTREAMER_ROOT_WINDOWS AND DEFINED ENV{GSTREAMER_1_0_ROOT_MSVC_X86_64})
  set(GSTREAMER_ROOT_WINDOWS "$ENV{GSTREAMER_1_0_ROOT_MSVC_X86_64}" CACHE PATH "Path to the GStreamer MSVC x86_64 SDK root" FORCE)
elseif(NOT GSTREAMER_ROOT_WINDOWS AND DEFINED ENV{GSTREAMER_1_0_ROOT_X86_64})
  set(GSTREAMER_ROOT_WINDOWS "$ENV{GSTREAMER_1_0_ROOT_X86_64}" CACHE PATH "Path to the GStreamer MSVC x86_64 SDK root" FORCE)
endif()

set(GST_AUDIO_GSTREAMER_ROOT_WINDOWS "${GSTREAMER_ROOT_WINDOWS}")
if(GST_AUDIO_GSTREAMER_ROOT_WINDOWS)
  file(TO_CMAKE_PATH "${GST_AUDIO_GSTREAMER_ROOT_WINDOWS}" GST_AUDIO_GSTREAMER_ROOT_WINDOWS)
endif()

# Define build configuration option.
get_property(IS_MULTICONFIG GLOBAL PROPERTY GENERATOR_IS_MULTI_CONFIG)
if(IS_MULTICONFIG)
  set(CMAKE_CONFIGURATION_TYPES "Debug;Profile;Release"
    CACHE STRING "" FORCE)
else()
  if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
    set(CMAKE_BUILD_TYPE "Debug" CACHE
      STRING "Flutter build mode" FORCE)
    set_property(CACHE CMAKE_BUILD_TYPE PROPERTY STRINGS
      "Debug" "Profile" "Release")
  endif()
endif()
# Define settings for the Profile build mode.
set(CMAKE_EXE_LINKER_FLAGS_PROFILE "${CMAKE_EXE_LINKER_FLAGS_RELEASE}")
set(CMAKE_SHARED_LINKER_FLAGS_PROFILE "${CMAKE_SHARED_LINKER_FLAGS_RELEASE}")
set(CMAKE_C_FLAGS_PROFILE "${CMAKE_C_FLAGS_RELEASE}")
set(CMAKE_CXX_FLAGS_PROFILE "${CMAKE_CXX_FLAGS_RELEASE}")

# Use Unicode for all projects.
add_definitions(-DUNICODE -D_UNICODE)

# Compilation settings that should be applied to most targets.
#
# Be cautious about adding new options here, as plugins use this function by
# default. In most cases, you should add new options to specific targets instead
# of modifying this function.
function(APPLY_STANDARD_SETTINGS TARGET)
  target_compile_features(${TARGET} PUBLIC cxx_std_17)
  target_compile_options(${TARGET} PRIVATE /W4 /WX /wd"4100")
  target_compile_options(${TARGET} PRIVATE /EHsc)
  target_compile_definitions(${TARGET} PRIVATE "_HAS_EXCEPTIONS=0")
  target_compile_definitions(${TARGET} PRIVATE "$<$<CONFIG:Debug>:_DEBUG>")
endfunction()

# Flutter library and tool build rules.
set(FLUTTER_MANAGED_DIR "${CMAKE_CURRENT_SOURCE_DIR}/flutter")
add_subdirectory(${FLUTTER_MANAGED_DIR})

# Application build; see runner/CMakeLists.txt.
add_subdirectory("runner")


# Generated plugin build rules, which manage building the plugins and adding
# them to the application.
include(flutter/generated_plugins.cmake)


# === Installation ===
# Support files are copied into place next to the executable, so that it can
# run in place. This is done instead of making a separate bundle (as on Linux)
# so that building and running from within Visual Studio will work.
set(BUILD_BUNDLE_DIR "$<TARGET_FILE_DIR:${BINARY_NAME}>")
# Make the "install" step default, as it's required to run.
set(CMAKE_VS_INCLUDE_INSTALL_TO_DEFAULT_BUILD 1)
if(CMAKE_INSTALL_PREFIX_INITIALIZED_TO_DEFAULT)
  set(CMAKE_INSTALL_PREFIX "${BUILD_BUNDLE_DIR}" CACHE PATH "..." FORCE)
endif()

set(INSTALL_BUNDLE_DATA_DIR "${CMAKE_INSTALL_PREFIX}/data")
set(INSTALL_BUNDLE_LIB_DIR "${CMAKE_INSTALL_PREFIX}")

install(TARGETS ${BINARY_NAME} RUNTIME DESTINATION "${CMAKE_INSTALL_PREFIX}"
  COMPONENT Runtime)

install(FILES "${FLUTTER_ICU_DATA_FILE}" DESTINATION "${INSTALL_BUNDLE_DATA_DIR}"
  COMPONENT Runtime)

install(FILES "${FLUTTER_LIBRARY}" DESTINATION "${INSTALL_BUNDLE_LIB_DIR}"
  COMPONENT Runtime)

if(GST_AUDIO_GSTREAMER_ROOT_WINDOWS)
  set(GST_AUDIO_GSTREAMER_BIN_DIR "${GST_AUDIO_GSTREAMER_ROOT_WINDOWS}/bin")
  set(GST_AUDIO_GSTREAMER_PLUGIN_DIR "${GST_AUDIO_GSTREAMER_ROOT_WINDOWS}/lib/gstreamer-1.0")
  set(GST_AUDIO_GSTREAMER_GIO_MODULE_DIR "${GST_AUDIO_GSTREAMER_ROOT_WINDOWS}/lib/gio/modules")
  set(GST_AUDIO_GSTREAMER_LIBEXEC_DIR "${GST_AUDIO_GSTREAMER_ROOT_WINDOWS}/libexec/gstreamer-1.0")

  if(EXISTS "${GST_AUDIO_GSTREAMER_BIN_DIR}")
    file(GLOB GST_AUDIO_GSTREAMER_RUNTIME_DLLS CONFIGURE_DEPENDS
      "${GST_AUDIO_GSTREAMER_BIN_DIR}/*.dll")
    if(GST_AUDIO_GSTREAMER_RUNTIME_DLLS)
      install(FILES ${GST_AUDIO_GSTREAMER_RUNTIME_DLLS}
        DESTINATION "${INSTALL_BUNDLE_LIB_DIR}"
        COMPONENT Runtime)
    else()
      message(WARNING "No GStreamer runtime DLLs found in: ${GST_AUDIO_GSTREAMER_BIN_DIR}")
    endif()
  else()
    message(WARNING "GStreamer Windows bin directory not found: ${GST_AUDIO_GSTREAMER_BIN_DIR}")
  endif()

  if(EXISTS "${GST_AUDIO_GSTREAMER_PLUGIN_DIR}")
    install(DIRECTORY "${GST_AUDIO_GSTREAMER_PLUGIN_DIR}/"
      DESTINATION "${INSTALL_BUNDLE_LIB_DIR}/lib/gstreamer-1.0"
      COMPONENT Runtime
      FILES_MATCHING PATTERN "*.dll")
  else()
    message(WARNING "GStreamer Windows plugin directory not found: ${GST_AUDIO_GSTREAMER_PLUGIN_DIR}")
  endif()

  if(EXISTS "${GST_AUDIO_GSTREAMER_GIO_MODULE_DIR}")
    install(DIRECTORY "${GST_AUDIO_GSTREAMER_GIO_MODULE_DIR}/"
      DESTINATION "${INSTALL_BUNDLE_LIB_DIR}/lib/gio/modules"
      COMPONENT Runtime
      FILES_MATCHING PATTERN "*.dll")
  endif()

  if(EXISTS "${GST_AUDIO_GSTREAMER_LIBEXEC_DIR}")
    install(DIRECTORY "${GST_AUDIO_GSTREAMER_LIBEXEC_DIR}/"
      DESTINATION "${INSTALL_BUNDLE_LIB_DIR}/libexec/gstreamer-1.0"
      COMPONENT Runtime)
  endif()
endif()

if(PLUGIN_BUNDLED_LIBRARIES)
  install(FILES "${PLUGIN_BUNDLED_LIBRARIES}"
    DESTINATION "${INSTALL_BUNDLE_LIB_DIR}"
    COMPONENT Runtime)
endif()

# Copy the native assets provided by the build.dart from all packages.
set(NATIVE_ASSETS_DIR "${PROJECT_BUILD_DIR}native_assets/windows/")
install(DIRECTORY "${NATIVE_ASSETS_DIR}"
   DESTINATION "${INSTALL_BUNDLE_LIB_DIR}"
   COMPONENT Runtime)

# Fully re-copy the assets directory on each build to avoid having stale files
# from a previous install.
set(FLUTTER_ASSET_DIR_NAME "flutter_assets")
install(CODE "
  file(REMOVE_RECURSE \"${INSTALL_BUNDLE_DATA_DIR}/${FLUTTER_ASSET_DIR_NAME}\")
  " COMPONENT Runtime)
install(DIRECTORY "${PROJECT_BUILD_DIR}/${FLUTTER_ASSET_DIR_NAME}"
  DESTINATION "${INSTALL_BUNDLE_DATA_DIR}" COMPONENT Runtime)

# Install the AOT library on non-Debug builds only.
install(FILES "${AOT_LIBRARY}" DESTINATION "${INSTALL_BUNDLE_DATA_DIR}"
  CONFIGURATIONS Profile;Release
  COMPONENT Runtime)
