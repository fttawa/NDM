// Test-only loopback seeder. Never included in the application resources.
#include "common.hpp"
#include <libtorrent/create_torrent.hpp>

int main(int argc, char** argv) {
    try {
        if (argc != 3) throw std::runtime_error("Usage: ndm-bt-fixture ROOT v1|v2|hybrid");
        auto root = fs::u8path(argv[1]); std::string mode = argv[2];
        auto seed = root / L"seed"; fs::create_directories(seed / L"pack/sub");
        std::vector<std::pair<std::string, int>> files{{"pack/a.bin", 1024 * 1024 + 47}, {"pack/sub/b.bin", 512 * 1024 + 13}};
        for (auto const& file : files) {
            std::ofstream out(seed / fs::u8path(file.first), std::ios::binary);
            for (int i = 0; i < file.second; ++i) out.put(static_cast<char>((i * 31 + i / 1024) % 251));
            if (!out) throw std::runtime_error("Cannot write fixture");
        }
        auto flags = mode == "v1" ? lt::create_torrent::v1_only : mode == "v2" ? lt::create_torrent::v2_only : lt::create_flags_t{};
        lt::create_torrent maker(lt::list_files((seed / L"pack").u8string()), 32768, flags);
        lt::set_piece_hashes(maker, seed.u8string());
        auto value = maker.generate(); value.dict().erase("creation date");
        std::vector<char> bytes; lt::bencode(std::back_inserter(bytes), value);
        auto params = lt::load_torrent_buffer(bytes);
        params.save_path = seed.u8string();
        params.flags &= ~(lt::torrent_flags::paused | lt::torrent_flags::auto_managed);
        lt::session session(settings(true)); auto handle = session.add_torrent(params);
        auto until = Clock::now() + std::chrono::seconds(15);
        while (!handle.status().is_seeding) {
            if (Clock::now() > until) throw std::runtime_error("Fixture seeder timed out");
            std::this_thread::sleep_for(std::chrono::milliseconds(20));
        }
        auto details = inspect(*params.ti);
        details["type"] = "fixture"; details["metainfo"] = encode64(bytes);
        details["url"] = lt::make_magnet_uri(params) + "&x.pe=127.0.0.1:" + std::to_string(session.listen_port());
        emit(details);
        std::string command; std::getline(std::cin, command);
        handle.pause(); session.pause(); session.remove_torrent(handle); return 0;
    } catch (std::exception const& error) { emit({{"type", "error"}, {"message", error.what()}}); return 1; }
}
