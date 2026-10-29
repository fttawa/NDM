#include "common.hpp"
#include <libtorrent/ip_filter.hpp>

// Windows pipe polling keeps all libtorrent calls on one thread and lets parent EOF
// stop a download without a resident command server or detached reader thread.
class Input {
    HANDLE pipe = GetStdHandle(STD_INPUT_HANDLE);
    std::string buffer;
    bool ended = false;
public:
    std::vector<json> poll(bool blocking = false, std::size_t max_events = SIZE_MAX) {
        std::vector<json> events;
        do {
            auto newline = buffer.find('\n');
            while (newline != std::string::npos && events.size() < max_events) {
                auto line = buffer.substr(0, newline); buffer.erase(0, newline + 1);
                if (!line.empty()) events.push_back(json::parse(line));
                newline = buffer.find('\n');
            }
            if (!events.empty()) return events;
            if (ended) return {json{{"type", "stop"}}};
            DWORD available = 0;
            if (!PeekNamedPipe(pipe, nullptr, 0, nullptr, &available, nullptr)) { ended = true; continue; }
            if (available == 0) {
                if (!blocking) return events;
                std::this_thread::sleep_for(std::chrono::milliseconds(10)); continue;
            }
            char bytes[8192]; DWORD read = 0;
            if (!ReadFile(pipe, bytes, (std::min)(available, DWORD(sizeof bytes)), &read, nullptr) || read == 0) { ended = true; continue; }
            buffer.append(bytes, read);
            if (buffer.size() > 24 * 1024 * 1024) throw std::runtime_error("Command exceeds size limit");
        } while (blocking || buffer.find('\n') != std::string::npos);
        return events;
    }
};

json details_for(lt::torrent_info const& ti, int version) {
    auto details = inspect(ti);
    if (version == 2 && ti.info_hashes().has_v2()) {
        details["infoHash"] = hex(ti.info_hashes().v2); details["version"] = 2;
    }
    return details;
}

std::vector<char> cache_metadata(lt::add_torrent_params params, std::vector<lt::announce_entry> const& original) {
    if (!params.ti) return {};
    // Supplemental trackers are settings, not part of the saved torrent identity.
    params.trackers.clear(); params.tracker_tiers.clear();
    for (auto const& tracker : original) { params.trackers.push_back(tracker.url); params.tracker_tiers.push_back(tracker.tier); }
    // Preserve the exact info bytes: re-encoding the info dictionary changes hashes.
    lt::entry value;
    try {
        value = lt::write_torrent_file(params, lt::write_flags::allow_missing_piece_layer);
    } catch (lt::system_error const&) {
        auto info = params.ti->info_section();
        value["info"] = lt::entry(lt::entry::preformatted_type(info.begin(), info.end()));
        for (auto const& tracker : params.trackers) {
            lt::entry tier(lt::entry::list_t); tier.list().emplace_back(tracker);
            value["announce-list"].list().push_back(std::move(tier));
        }
    }
    std::vector<char> raw; lt::bencode(std::back_inserter(raw), value);
    // BEP 9 omits BEP 52 piece layers. Never persist incomplete placeholder layers;
    // keeping the authentic info section lets libtorrent fetch the hashes on resume.
    try { lt::load_torrent_buffer(raw); }
    catch (lt::system_error const&) {
        value.dict().erase("piece layers"); raw.clear();
        lt::bencode(std::back_inserter(raw), value);
    }
    if (raw.size() > max_meta) throw std::runtime_error("Torrent exceeds 16 MiB");
    return raw;
}

int rate(json const& value) {
    return static_cast<int>((std::min)(value.get<std::uint64_t>(), std::uint64_t(INT_MAX)));
}

lt::settings_pack bt_settings(json const& bt, bool local) {
    lt::settings_pack s;
    s.set_bool(lt::settings_pack::enable_dht, !local && bt.value("dht", true));
    s.set_bool(lt::settings_pack::enable_lsd, !local && bt.value("lsd", false));
    s.set_bool(lt::settings_pack::enable_upnp, !local && bt.value("upnp", false));
    s.set_bool(lt::settings_pack::enable_natpmp, !local && bt.value("upnp", false));
    s.set_int(lt::settings_pack::connections_limit, std::clamp(bt.value("maxPeers", 80), 10, 500));
    s.set_int(lt::settings_pack::upload_rate_limit, rate(json(bt.value("uploadLimit", std::uint64_t(0)) * 1024)));
    if (bt.contains("bootstrapNodes")) {
        std::string nodes;
        for (auto const& node : bt["bootstrapNodes"]) { if (!nodes.empty()) nodes += ','; nodes += node.get<std::string>(); }
        s.set_str(lt::settings_pack::dht_bootstrap_nodes, nodes);
    }
    return s;
}

json tracker_details(lt::torrent_handle const& handle) {
    json result = json::array();
    for (auto const& tracker : handle.trackers()) {
        bool updating = false, working = false; std::string message;
        for (auto const& endpoint : tracker.endpoints) for (auto const& hash : endpoint.info_hashes) {
            updating |= hash.updating;
            // A successful event=paused (metadata authorization) doesn't set
            // start_sent. Its response still sets min_announce. Count any
            // successful interface even if other interfaces cannot reach it.
            working |= tracker.verified && !hash.last_error && hash.min_announce != lt::time_point32::min();
            if (hash.last_error) message = hash.message.empty() ? hash.last_error.message() : hash.message;
        }
        result.push_back({{"url", tracker.url}, {"state", working ? "working" : updating ? "announcing" : message.empty() ? "waiting" : "error"}, {"message", working ? "" : message}});
    }
    return result;
}

void run(json const& config, Input& input) {
    auto hash = config.at("hash").get<std::string>();
    int version = config.value("version", hash.size() == 64 ? 2 : 1);
    auto directory = config.at("directory").get<std::string>();
    auto root = fs::u8path(directory);
    auto params = lt::parse_magnet_uri(config.at("url").get<std::string>());
    auto peers = params.peers;
    if (!config.value("metainfo", std::string{}).empty()) {
        auto from_file = lt::load_torrent_buffer(decode64(config["metainfo"].get<std::string>()));
        from_file.trackers.insert(from_file.trackers.end(), params.trackers.begin(), params.trackers.end());
        // Both URL and cached file hash must agree before opening content files.
        auto details = details_for(*from_file.ti, version);
        if (details["infoHash"] != hash || details["version"] != version)
            throw std::runtime_error("Cached torrent hash mismatch");
        safe_output(root, details, hash);
        params = std::move(from_file);
        params.file_priorities.assign(params.ti->num_files(), lt::dont_download);
    }
    auto s = settings(config.value("localTest", false));
    s.set_int(lt::settings_pack::download_rate_limit, rate(config.value("limit", json(0))));
    lt::session session(s);
    bool local = config.value("localTest", false);
    json bt = config.value("bt", json::object());
    session.apply_settings(bt_settings(bt, local));
    // Apply the global limiter to LAN peers too, using the supported peer-class API.
    lt::ip_filter filter;
    filter.add_rule(lt::make_address("0.0.0.0"), lt::make_address("255.255.255.255"), 1);
    filter.add_rule(lt::make_address("::"), lt::make_address("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"), 1);
    session.set_peer_class_filter(filter);
    params.save_path = directory;
    params.flags &= ~(lt::torrent_flags::paused | lt::torrent_flags::auto_managed);
    params.flags |= lt::torrent_flags::upload_mode | lt::torrent_flags::default_dont_download;
    auto handle = session.add_torrent(params);
    auto original_trackers = handle.trackers();
    auto apply_torrent_settings = [&] {
        bool priv = handle.torrent_file() && handle.torrent_file()->priv();
        if (!bt.value("pex", true) || priv || local) handle.set_flags(lt::torrent_flags::disable_pex);
        else handle.unset_flags(lt::torrent_flags::disable_pex);
        if (!bt.value("dht", true) || priv || local) handle.set_flags(lt::torrent_flags::disable_dht);
        else handle.unset_flags(lt::torrent_flags::disable_dht);
        auto trackers = original_trackers; std::set<std::string> seen;
        for (auto const& tracker : trackers) seen.insert(tracker.url);
        if (!priv && !local && bt.contains("trackers")) for (auto const& value : bt["trackers"]) {
            auto url = value.get<std::string>();
            if (seen.insert(url).second) { lt::announce_entry entry(url); entry.tier = 127; entry.source = lt::announce_entry::source_client; trackers.push_back(entry); }
        }
        auto current = handle.trackers();
        bool changed = current.size() != trackers.size();
        for (std::size_t i = 0; !changed && i < current.size(); ++i) changed = current[i].url != trackers[i].url;
        if (changed) handle.replace_trackers(trackers);
    };
    apply_torrent_settings();
    auto connect = [&] { for (auto const& peer : peers) handle.connect_peer(peer); };
    connect();
    bool authorized = false, metadata_sent = false, requested = false;
    json details;
    auto deadline = Clock::now() + std::chrono::seconds(180);
    auto progress_at = Clock::time_point::min();
    auto details_at = Clock::time_point::min();
    int dht_nodes = 0;
    auto metadata = [&](lt::add_torrent_params const& p) {
        if (!p.ti) return false;
        auto candidate = details_for(*p.ti, version);
        if (candidate["infoHash"] != hash || candidate["version"] != version)
            throw std::runtime_error("Peer metadata hash mismatch");
        auto raw = cache_metadata(p, original_trackers);
        if (raw.empty()) return false;
        details = std::move(candidate);
        emit({{"type", "metadata"}, {"metainfo", encode64(raw)}}); return true;
    };
    auto finish = [&] {
        handle.pause(); session.pause();
        handle.save_resume_data(lt::torrent_handle::save_info_dict | lt::torrent_handle::flush_disk_cache);
        auto until = Clock::now() + std::chrono::seconds(3);
        while (Clock::now() < until) {
            std::vector<lt::alert*> alerts; session.pop_alerts(&alerts);
            for (auto alert : alerts) {
                if (auto saved = lt::alert_cast<lt::save_resume_data_alert>(alert)) {
                    metadata(saved->params); session.remove_torrent(handle); return;
                }
                if (lt::alert_cast<lt::save_resume_data_failed_alert>(alert)) {
                    session.remove_torrent(handle); return;
                }
            }
            session.wait_for_alert(std::chrono::milliseconds(50));
        }
        session.remove_torrent(handle);
    };
    while (true) {
        auto commands = input.poll(); bool stop = false;
        for (auto const& command : commands) if (command.at("type") == "stop") stop = true;
        if (stop) { finish(); emit({{"type", "stopped"}}); return; }
        for (auto const& command : commands) {
            if (command.at("type") == "limit") {
                lt::settings_pack change; change.set_int(lt::settings_pack::download_rate_limit, rate(command.at("bps")));
                session.apply_settings(change);
            } else if (command.at("type") == "configure") {
                bt = command.at("bt"); session.apply_settings(bt_settings(bt, local));
                if (!bt.value("dht", true)) dht_nodes = 0;
                apply_torrent_settings(); details_at = Clock::time_point::min();
            } else if (command.at("type") == "start" && metadata_sent && !authorized) {
                safe_output(root, details, hash);
                handle.prioritize_files(std::vector<lt::download_priority_t>(handle.torrent_file()->num_files(), lt::default_priority));
                handle.unset_flags(lt::torrent_flags::upload_mode);
                connect(); authorized = true;
            }
        }
        std::vector<lt::alert*> alerts; session.pop_alerts(&alerts);
        for (auto alert : alerts) {
            if (auto stats = lt::alert_cast<lt::dht_stats_alert>(alert)) {
                dht_nodes = 0; for (auto const& bucket : stats->routing_table) dht_nodes += bucket.num_nodes;
            } else if (auto saved = lt::alert_cast<lt::save_resume_data_alert>(alert)) {
                if (!metadata_sent) {
                    metadata_sent = metadata(saved->params);
                    if (!metadata_sent) throw std::runtime_error("Cannot cache torrent metadata");
                }
            } else if (auto failed = lt::alert_cast<lt::save_resume_data_failed_alert>(alert)) {
                throw std::runtime_error(failed->message());
            } else if (auto failure = lt::alert_cast<lt::torrent_error_alert>(alert)) {
                throw std::runtime_error(failure->message());
            }
        }
        auto status = handle.status();
        if (status.errc) throw std::runtime_error(status.errc.message());
        if (status.has_metadata && !requested) {
            // Validate all paths before authorizing any content download.
            details_for(*handle.torrent_file(), version);
            apply_torrent_settings();
            handle.save_resume_data(lt::torrent_handle::save_info_dict); requested = true;
        }
        if (!metadata_sent && Clock::now() > deadline) throw std::runtime_error("Magnet metadata timed out; no metadata peer found");
        if (authorized && status.is_seeding) { finish(); emit({{"type", "complete"}}); return; }
        auto now = Clock::now();
        if (progress_at == Clock::time_point::min() || now - progress_at >= std::chrono::milliseconds(250)) {
            bool checking = status.state == lt::torrent_status::checking_files || status.state == lt::torrent_status::checking_resume_data;
            json event{{"type", "progress"}, {"downloaded", status.total_wanted_done},
                {"speed", checking ? 0 : status.download_payload_rate}, {"peers", status.num_peers}, {"checking", checking}};
            if (details_at == Clock::time_point::min() || now - details_at >= std::chrono::seconds(1)) {
                bool priv = status.has_metadata && handle.torrent_file()->priv();
                json files = json::array();
                if (status.has_metadata) {
                    auto done = handle.file_progress(lt::torrent_handle::piece_granularity);
                    auto const& layout = handle.torrent_file()->layout();
                    for (auto index : layout.file_range()) if (!(layout.file_flags(index) & lt::file_storage::flag_pad_file)) files.push_back(done[static_cast<std::size_t>(static_cast<int>(index))]);
                }
                event["details"] = {{"phase", !status.has_metadata ? "metadata" : checking ? "checking" : authorized ? "downloading" : "preparing"},
                    {"peers", status.num_peers}, {"seeds", status.num_seeds}, {"uploadSpeed", status.upload_payload_rate},
                    {"uploaded", status.total_payload_upload}, {"dhtNodes", dht_nodes}, {"pieces", status.num_pieces},
                    {"totalPieces", status.has_metadata ? handle.torrent_file()->num_pieces() : 0}, {"fileProgress", files},
                    {"dht", !priv && !local && bt.value("dht", true)}, {"pex", !priv && !local && bt.value("pex", true)},
                    {"private", priv}, {"maxPeers", bt.value("maxPeers", 80)}, {"uploadLimit", bt.value("uploadLimit", 0)},
                    {"trackers", tracker_details(handle)}};
                if (!local && bt.value("dht", true)) session.post_dht_stats();
                details_at = now;
            }
            emit(event);
            progress_at = now;
        }
        session.wait_for_alert(std::chrono::milliseconds(50));
    }
}

int main() {
    try {
        Input input; auto commands = input.poll(true, 1);
        if (commands.size() != 1) throw std::runtime_error("Expected one initialization command");
        auto const& request = commands.front();
        if (request.at("type") == "inspect") {
            auto params = lt::load_torrent_buffer(decode64(request.at("metainfo").get<std::string>()));
            auto details = details_for(*params.ti, request.value("version", 0));
            details["trackers"] = params.trackers; emit(details);
        } else if (request.at("type") == "run") run(request, input);
        else throw std::runtime_error("Unknown engine command");
        return 0;
    } catch (std::exception const& error) {
        emit({{"type", "error"}, {"message", error.what()}}); return 1;
    }
}
