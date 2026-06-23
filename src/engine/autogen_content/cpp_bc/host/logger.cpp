// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#include "logger.h"

#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <ctime>

#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#else
#include <limits.h>
#include <sys/stat.h>
#include <unistd.h>
#endif


namespace {

/// Directory holding the running executable, or "." if it cannot be found.
std::string exe_dir()
{
#ifdef _WIN32
    char buf[MAX_PATH];
    const DWORD n = GetModuleFileNameA(nullptr, buf, sizeof(buf));
    if (n == 0 || n >= sizeof(buf)) {
        return ".";
    }
    const std::string p(buf, n);
    const std::string::size_type slash = p.find_last_of("\\/");
    return (slash == std::string::npos) ? std::string(".") : p.substr(0, slash);
#else
    char buf[PATH_MAX];
    const ssize_t n = ::readlink("/proc/self/exe", buf, sizeof(buf) - 1);
    if (n <= 0) {
        return ".";
    }
    buf[n] = '\0';
    const std::string p(buf);
    const std::string::size_type slash = p.find_last_of('/');
    return (slash == std::string::npos) ? std::string(".") : p.substr(0, slash);
#endif
}

/// Create `dir` if missing. Existing-directory errors are ignored on purpose.
void make_dir(const std::string& dir)
{
#ifdef _WIN32
    CreateDirectoryA(dir.c_str(), nullptr);
#else
    ::mkdir(dir.c_str(), 0755);
#endif
}

/// True if `path` exists and is a directory.
bool dir_exists(const std::string& path)
{
#ifdef _WIN32
    const DWORD attr = GetFileAttributesA(path.c_str());
    return (attr != INVALID_FILE_ATTRIBUTES) &&
           ((attr & FILE_ATTRIBUTE_DIRECTORY) != 0);
#else
    struct stat st;
    return (::stat(path.c_str(), &st) == 0) && S_ISDIR(st.st_mode);
#endif
}

char path_sep()
{
#ifdef _WIN32
    return '\\';
#else
    return '/';
#endif
}

/// Pick the directory that per-run log files live in. Resolution order:
///   1. $EXHIBITION_LOG_DIR, when set and non-empty (operator override).
///   2. A ramdisk under /media/ramdisk on POSIX, when present -- keeps the
///      flush path off the spinning array so a momentary disk stall cannot
///      back up into the bus-servicing loop.
///   3. `<exe_dir>/logs` as the portable fallback.
/// The chosen directory (and any needed parent) is created before returning.
std::string resolve_log_dir()
{
    if (char const* env = std::getenv("EXHIBITION_LOG_DIR")) {
        if (env[0] != '\0') {
            make_dir(env);
            return std::string(env);
        }
    }
#ifndef _WIN32
    if (dir_exists("/media/ramdisk")) {
        const std::string dir = "/media/ramdisk/exhibition_bc/logs";
        make_dir("/media/ramdisk/exhibition_bc");
        make_dir(dir);
        if (dir_exists(dir)) {
            return dir;
        }
    }
#endif
    const std::string dir = exe_dir() + path_sep() + "logs";
    make_dir(dir);
    return dir;
}

/// Fill `out` with the current local time broken down into a std::tm.
void local_now(std::tm& out, std::time_t& t_out)
{
    t_out = std::chrono::system_clock::to_time_t(std::chrono::system_clock::now());
#ifdef _WIN32
    localtime_s(&out, &t_out);
#else
    localtime_r(&t_out, &out);
#endif
}

} // namespace


Logger::Logger(const std::string& path)
    : m_stop(false)
    , m_out(path.c_str(), std::ios::out | std::ios::trunc)
    , m_metrics_out(metrics_path_for(path).c_str(), std::ios::out | std::ios::trunc)
    , m_path(path)
    , m_open(false)
    , m_metrics_open(false)
{
    m_open = m_out.is_open();
    m_metrics_open = m_metrics_out.is_open();
    if (m_open) {
        // Start the background writer only once the LOG file is known good; if
        // its open failed, line()/hex()/metric() short-circuit and no thread is
        // needed. The metrics file is a best-effort side-car drained by the same
        // thread, so it rides along whenever the log is up.
        m_writer = std::thread(&Logger::writer_loop, this);
    }
}


Logger::~Logger()
{
    if (m_writer.joinable()) {
        {
            std::lock_guard<std::mutex> lock(m_mutex);
            m_stop = true;
        }
        m_cv.notify_one();
        m_writer.join();   // Drains both queues before returning.
    }
    if (m_out.is_open()) {
        m_out.flush();
    }
    if (m_metrics_out.is_open()) {
        m_metrics_out.flush();
    }
}


std::string Logger::metrics_path_for(const std::string& log_path)
{
    const std::string suffix = ".log";
    if (log_path.size() >= suffix.size() &&
        log_path.compare(log_path.size() - suffix.size(), suffix.size(), suffix) == 0) {
        return log_path.substr(0, log_path.size() - suffix.size()) + ".metrics.csv";
    }
    return log_path + ".metrics.csv";
}


void Logger::writer_loop()
{
    std::unique_lock<std::mutex> lock(m_mutex);
    for (;;) {
        m_cv.wait(lock, [this] {
            return !m_queue.empty() || !m_metrics_queue.empty() || m_stop;
        });

        // Drain everything queued so far. Swap each batch out and write it with
        // the lock released so producers never block on disk I/O.
        while (!m_queue.empty() || !m_metrics_queue.empty()) {
            std::deque<std::string> batch;
            std::deque<std::string> metrics_batch;
            batch.swap(m_queue);
            metrics_batch.swap(m_metrics_queue);
            lock.unlock();
            for (const std::string& s : batch) {
                m_out << s;
            }
            if (!batch.empty()) {
                m_out.flush();
            }
            for (const std::string& s : metrics_batch) {
                m_metrics_out << s;
            }
            if (!metrics_batch.empty()) {
                m_metrics_out.flush();
            }
            lock.lock();
        }

        if (m_stop) {
            return;
        }
    }
}


std::string Logger::default_log_path()
{
    const std::string logs = resolve_log_dir();

    std::tm tmv{};
    std::time_t t = 0;
    local_now(tmv, t);

    char stamp[32];
    std::strftime(stamp, sizeof(stamp), "%Y%m%d_%H%M%S", &tmv);

    return logs + path_sep() + "exhibition_bc_" + stamp + ".log";
}


bool Logger::is_open() const
{
    return m_open;
}


const std::string& Logger::path() const
{
    return m_path;
}


std::string Logger::timestamp() const
{
    using std::chrono::duration_cast;
    using std::chrono::milliseconds;
    using std::chrono::system_clock;

    const auto now = system_clock::now();
    const std::time_t t = system_clock::to_time_t(now);
    const auto ms = duration_cast<milliseconds>(now.time_since_epoch()) % 1000;

    std::tm tmv{};
#ifdef _WIN32
    localtime_s(&tmv, &t);
#else
    localtime_r(&t, &tmv);
#endif

    char date[24];
    std::strftime(date, sizeof(date), "%Y-%m-%d %H:%M:%S", &tmv);

    // Local UTC offset as ±HHMM (e.g. -0700) so the wall-clock time is
    // unambiguous to a later reader.
    char offset[8];
    std::strftime(offset, sizeof(offset), "%z", &tmv);

    char out[48];
    std::snprintf(out, sizeof(out), "%s.%03d%s",
                  date, static_cast<int>(ms.count()), offset);
    return std::string(out);
}


std::string Logger::compose(char const* level, const std::string& body) const
{
    std::string s = timestamp();
    s += " [";
    s += level;
    s += "] ";
    s += body;
    s += '\n';
    return s;
}


void Logger::enqueue(std::string&& formatted)
{
    {
        std::lock_guard<std::mutex> lock(m_mutex);
        m_queue.push_back(std::move(formatted));
    }
    m_cv.notify_one();
}


void Logger::line(char const* level, const std::string& msg)
{
    if (!m_open) {
        return;
    }
    enqueue(compose(level, msg));
}


void Logger::metric(const std::string& csv_line)
{
    if (!m_metrics_open) {
        return;
    }
    std::string formatted = csv_line;
    formatted += '\n';
    {
        std::lock_guard<std::mutex> lock(m_mutex);
        m_metrics_queue.push_back(std::move(formatted));
    }
    m_cv.notify_one();
}


void Logger::hex(char const* level, char const* prefix,
                 uint8_t const* data, uint32_t len)
{
    if (!m_open || data == nullptr) {
        return;
    }

    // Format every wrapped line first, then hand the whole group to the queue
    // under a single lock so a hex dump can't interleave with another thread's
    // lines. Each line is stamped with its own timestamp at format time.
    constexpr uint32_t kPerLine = 16;
    std::deque<std::string> lines;
    for (uint32_t off = 0; off < len; off += kPerLine) {
        const uint32_t end = (off + kPerLine < len) ? (off + kPerLine) : len;
        std::string bytes;
        char b[4];
        for (uint32_t i = off; i < end; ++i) {
            std::snprintf(b, sizeof(b), "%02X ", data[i]);
            bytes += b;
        }
        std::string body(prefix);
        body += bytes;
        lines.push_back(compose(level, body));
    }

    {
        std::lock_guard<std::mutex> lock(m_mutex);
        for (std::string& s : lines) {
            m_queue.push_back(std::move(s));
        }
    }
    m_cv.notify_one();
}
