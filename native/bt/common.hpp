#pragma once
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <libtorrent/add_torrent_params.hpp>
#include <libtorrent/alert_types.hpp>
#include <libtorrent/bencode.hpp>
#include <libtorrent/entry.hpp>
#include <libtorrent/file_storage.hpp>
#include <libtorrent/hex.hpp>
#include <libtorrent/load_torrent.hpp>
#include <libtorrent/magnet_uri.hpp>
#include <libtorrent/session.hpp>
#include <libtorrent/settings_pack.hpp>
#include <libtorrent/torrent_handle.hpp>
#include <libtorrent/torrent_info.hpp>
#include <libtorrent/torrent_status.hpp>
#include <libtorrent/write_resume_data.hpp>
#include <nlohmann/json.hpp>
#include <algorithm>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <limits>
#include <set>
#include <sstream>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>
#include <windows.h>

namespace lt = libtorrent;
namespace fs = std::filesystem;
using json = nlohmann::json;
using Clock = std::chrono::steady_clock;
constexpr std::size_t max_meta = 16 * 1024 * 1024;

inline void emit(json event) { std::cout << event.dump() << std::endl; }
inline std::string lower(std::string text) {
    for (char& c : text) if (c >= 'A' && c <= 'Z') c += 'a' - 'A';
    return text;
}
inline std::string hex(lt::sha1_hash const& hash) { return lt::aux::to_hex(hash); }
inline std::string hex(lt::sha256_hash const& hash) { return lt::aux::to_hex(hash); }
inline std::string encode64(std::vector<char> const& bytes) {
    static char const alphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    std::string out;
    out.reserve((bytes.size() + 2) / 3 * 4);
    for (std::size_t i = 0; i < bytes.size(); i += 3) {
        unsigned n = static_cast<unsigned char>(bytes[i]) << 16;
        if (i + 1 < bytes.size()) n |= static_cast<unsigned char>(bytes[i + 1]) << 8;
        if (i + 2 < bytes.size()) n |= static_cast<unsigned char>(bytes[i + 2]);
        out += alphabet[(n >> 18) & 63]; out += alphabet[(n >> 12) & 63];
        out += i + 1 < bytes.size() ? alphabet[(n >> 6) & 63] : '=';
        out += i + 2 < bytes.size() ? alphabet[n & 63] : '=';
    }
    return out;
}
inline std::vector<char> decode64(std::string const& text) {
    if (text.empty() || text.size() > (max_meta + 2) / 3 * 4 || text.size() % 4)
        throw std::runtime_error("Invalid torrent metadata length");
    std::vector<char> out;
    out.reserve(text.size() / 4 * 3);
    for (std::size_t i = 0; i < text.size(); i += 4) {
        unsigned n = 0;
        for (std::size_t j = 0; j < 4; ++j) {
            char c = text[i + j]; int v = -1;
            if (c >= 'A' && c <= 'Z') v = c - 'A';
            else if (c >= 'a' && c <= 'z') v = c - 'a' + 26;
            else if (c >= '0' && c <= '9') v = c - '0' + 52;
            else if (c == '+') v = 62; else if (c == '/') v = 63;
            else if (c == '=' && i + 4 == text.size() && j >= 2) v = 0;
            if (v < 0 || (j == 3 && text[i + 2] == '=' && c != '='))
                throw std::runtime_error("Invalid base64 metadata");
            n = (n << 6) | v;
        }
        out.push_back(static_cast<char>(n >> 16));
        if (text[i + 2] != '=') out.push_back(static_cast<char>(n >> 8));
        if (text[i + 3] != '=') out.push_back(static_cast<char>(n));
    }
    if (out.size() > max_meta) throw std::runtime_error("Torrent exceeds 16 MiB");
    return out;
}
inline void validate_path(std::string const& name) {
    if (name.empty() || name.front() == '/' || name.front() == '\\')
        throw std::runtime_error("Torrent contains an absolute or empty path");
    std::string normalized = name;
    std::replace(normalized.begin(), normalized.end(), '\\', '/');
    std::istringstream path(normalized); std::string part; bool first = true;
    while (std::getline(path, part, '/')) {
        if (part.empty() || part == "." || part == ".." || part.back() == '.' || part.back() == ' ')
            throw std::runtime_error("Torrent contains an unsafe file path");
        for (unsigned char c : part) if (c < 32 || std::string("<>:\"|?*").find(c) != std::string::npos)
            throw std::runtime_error("Torrent contains an unsafe Windows filename");
        auto stem = lower(part.substr(0, part.find('.')));
        if (stem == "con" || stem == "prn" || stem == "aux" || stem == "nul"
            || (stem.size() == 4 && (stem.substr(0, 3) == "com" || stem.substr(0, 3) == "lpt")
                && stem[3] >= '1' && stem[3] <= '9') || (first && lower(part) == ".ndm-torrent"))
            throw std::runtime_error("Torrent contains a reserved Windows filename");
        first = false;
    }
    if (normalized.back() == '/') throw std::runtime_error("Torrent filename ends in a separator");
}
inline json inspect(lt::torrent_info const& ti) {
    auto const& hashes = ti.info_hashes();
    json result{{"type", "inspection"}, {"name", ti.name()}, {"version", hashes.has_v1() ? 1 : 2},
        {"infoHash", hashes.has_v1() ? hex(hashes.v1) : hex(hashes.v2)}, {"total", 0}, {"files", json::array()},
        {"fileSizes", json::array()}, {"trackers", json::array()}, {"private", ti.priv()},
        {"pieceLength", ti.piece_length()}, {"pieceCount", ti.num_pieces()}};
    auto const& files = ti.layout(); std::set<std::wstring> names; std::uint64_t total = 0;
    for (auto index : files.file_range()) {
        auto flags = files.file_flags(index);
        if (flags & lt::file_storage::flag_symlink) throw std::runtime_error("Torrent symlinks are unsupported");
        if (flags & lt::file_storage::flag_pad_file) continue;
        auto name = files.file_path(index); std::replace(name.begin(), name.end(), '\\', '/');
        validate_path(name);
        auto wide = fs::u8path(name).wstring();
        if (!wide.empty()) CharLowerBuffW(wide.data(), static_cast<DWORD>(wide.size()));
        if (!names.insert(wide).second) throw std::runtime_error("Torrent contains duplicate filenames");
        auto size = files.file_size(index);
        if (size < 0 || std::uint64_t(size) > std::numeric_limits<std::uint64_t>::max() - total)
            throw std::runtime_error("Torrent size overflow");
        total += size; result["files"].push_back(name); result["fileSizes"].push_back(size);
    }
    if (result["files"].empty()) throw std::runtime_error("Torrent has no content files");
    result["total"] = total; return result;
}
inline void reject_link(fs::path const& path) {
    DWORD attrs = GetFileAttributesW(path.c_str());
    if (attrs == INVALID_FILE_ATTRIBUTES) {
        auto error = GetLastError();
        if (error != ERROR_FILE_NOT_FOUND && error != ERROR_PATH_NOT_FOUND)
            throw std::runtime_error("Cannot inspect torrent output path");
    } else if (attrs & FILE_ATTRIBUTE_REPARSE_POINT) throw std::runtime_error("Torrent output contains a link or junction");
}
inline void safe_output(fs::path const& root, json const& details, std::string const& hash) {
    reject_link(root); reject_link(root / L".ndm-torrent");
    std::ifstream marker(root / L".ndm-torrent", std::ios::binary);
    std::string saved(65, '\0'); marker.read(saved.data(), 65); saved.resize(static_cast<std::size_t>(marker.gcount()));
    if (saved != hash) throw std::runtime_error("Torrent directory ownership mismatch");
    for (auto const& file : details["files"]) {
        auto path = root;
        for (auto const& part : fs::u8path(file.get<std::string>())) { path /= part; reject_link(path); }
    }
}
inline lt::settings_pack settings(bool local) {
    lt::settings_pack s;
    s.set_str(lt::settings_pack::listen_interfaces, local ? "127.0.0.1:0" : "0.0.0.0:0,[::]:0");
    s.set_bool(lt::settings_pack::enable_dht, !local);
    s.set_bool(lt::settings_pack::enable_lsd, false);
    s.set_bool(lt::settings_pack::enable_upnp, false);
    s.set_bool(lt::settings_pack::enable_natpmp, false);
    s.set_bool(lt::settings_pack::announce_to_all_trackers, true);
    s.set_int(lt::settings_pack::connections_limit, 80);
    s.set_int(lt::settings_pack::alert_mask, lt::alert_category::error | lt::alert_category::status | lt::alert_category::storage);
    s.set_str(lt::settings_pack::user_agent, "NDM/libtorrent");
    return s;
}
