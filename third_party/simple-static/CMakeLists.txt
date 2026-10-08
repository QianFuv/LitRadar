cmake_minimum_required(VERSION 3.19)
project(litradar_simple LANGUAGES CXX)

if(NOT SIMPLE_SOURCE_DIR OR NOT SQLITE_DRIVER_DIR)
    message(FATAL_ERROR "SIMPLE_SOURCE_DIR and SQLITE_DRIVER_DIR are required")
endif()
if(MSVC)
    message(FATAL_ERROR "Use the same GNU-compatible compiler as Go cgo")
endif()

include("${SIMPLE_SOURCE_DIR}/contrib/CMakeRC.cmake")
cmrc_add_resource_library(PINYIN_TEXT TYPE OBJECT NAMESPACE pinyin_text)
cmrc_add_resources(PINYIN_TEXT WHENCE "${SIMPLE_SOURCE_DIR}" "${SIMPLE_SOURCE_DIR}/contrib/pinyin.txt")
set_property(TARGET PINYIN_TEXT PROPERTY POSITION_INDEPENDENT_CODE ON)

add_library(simple STATIC
    "${SIMPLE_SOURCE_DIR}/src/entry.cc"
    "${SIMPLE_SOURCE_DIR}/src/simple_tokenizer.cc"
    "${SIMPLE_SOURCE_DIR}/src/simple_highlight.cc"
    "${SIMPLE_SOURCE_DIR}/src/pinyin.cc"
    $<TARGET_OBJECTS:PINYIN_TEXT>
)
target_include_directories(simple BEFORE PRIVATE "${SQLITE_DRIVER_DIR}" "${SIMPLE_SOURCE_DIR}/src")
target_compile_definitions(simple PRIVATE SQLITE_CORE)
target_link_libraries(simple PRIVATE cmrc::base)
set_target_properties(simple PROPERTIES CXX_STANDARD 14 CXX_STANDARD_REQUIRED YES POSITION_INDEPENDENT_CODE ON)
