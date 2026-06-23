// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

#ifndef LOGGER_H
#define LOGGER_H

#include <stdint.h>

#include <condition_variable>
#include <deque>
#include <fstream>
#include <mutex>
#include <string>
#include <thread>


/// @file logger.h
/// @brief Cross-platform timestamped file logger for the BC host program.
/// @details
/// Writes one line per event, each prefixed with a local wall-clock timestamp
/// that carries the UTC offset, e.g.
/// `2026-05-27 14:12:30.044-0700 [INFO]  message`.
///
/// Disk I/O runs on a dedicated background writer thread: line() and hex()
/// format the text (stamping the timestamp at call time), append it to an
/// in-memory queue, and return without ever touching the filesystem. A
/// single writer thread drains the queue to the backing std::ofstream and
/// flushes after each batch. This keeps logging off the caller's critical
/// path -- the generated RtClient polls the bus from the same thread, and a
/// momentary filesystem stall used to block that loop long enough to trip the
/// per-transaction watchdog and report a spurious Timeout.
///
/// A mutex guards the queue: the generated RtClient is single-threaded, but
/// the guard also makes a shared Logger safe to hand to other producer
/// threads. Lines queued but not yet drained when the process is killed
/// (e.g. via exit()) may be lost; a clean destruction drains and flushes
/// everything.
class Logger {
    public:

    /// @brief Open `path` for writing, truncating any existing file.
    /// @param path Destination log-file path.
    explicit Logger(const std::string& path);
    ~Logger();

    Logger(const Logger&) = delete;
    Logger& operator=(const Logger&) = delete;

    /// @brief Build the default per-run log path
    ///        `<log_dir>/exhibition_bc_<YYYYMMDD_HHMMSS>.log`.
    /// @details The log directory is resolved in order: $EXHIBITION_LOG_DIR if
    ///          set; else a ramdisk under /media/ramdisk on POSIX when present
    ///          (so a disk stall cannot back up into the bus loop); else
    ///          `<exe_dir>/logs`. The chosen directory is created if needed.
    ///          The filename stamp is offset-free local time so it stays a
    ///          valid, portable filename; the UTC offset lives inside the log
    ///          lines.
    static std::string default_log_path();

    /// @brief True if the underlying file opened successfully.
    bool is_open() const;

    /// @brief Path the logger is writing to.
    const std::string& path() const;

    /// @brief Write one timestamped line: `<ts> [LEVEL] msg`.
    /// @param level Short level tag, e.g. "INFO", "WARN", "ERROR", "DEBUG".
    /// @param msg Message text.
    void line(char const* level, const std::string& msg);

    /// @brief Write a hex dump of `data`, wrapped at 16 bytes per line, each
    ///        line timestamped and prefixed with `prefix`.
    /// @param level Level tag for the dump lines.
    /// @param prefix Text placed before the hex bytes (e.g. "  bytes: ").
    /// @param data Buffer to dump (may be null when `len` is 0).
    /// @param len Number of bytes to dump.
    void hex(char const* level, char const* prefix,
             uint8_t const* data, uint32_t len);

    /// @brief Write one raw line to the side-car metrics file (no timestamp or
    ///        level prefix), e.g. a CSV row. A trailing newline is appended.
    /// @details Non-blocking, exactly like line()/hex(): the line is appended to
    ///          an in-memory queue and drained to disk by the same background
    ///          writer thread, so callers on the bus-servicing path never stall
    ///          on the filesystem. No-op if the metrics file failed to open.
    /// @param csv_line One record, without a trailing newline.
    void metric(const std::string& csv_line);

    private:

    /// @brief Format the current instant as `YYYY-MM-DD HH:MM:SS.mmm±HHMM`.
    std::string timestamp() const;

    /// @brief Build one fully-formatted, newline-terminated log line:
    ///        `<ts> [LEVEL] <body>\n`. Stamps the timestamp now.
    std::string compose(char const* level, const std::string& body) const;

    /// @brief Append an already-formatted line to the queue and wake the
    ///        writer thread. Non-blocking: never touches the filesystem.
    void enqueue(std::string&& formatted);

    /// @brief Derive the metrics-file path from the log path: a trailing `.log`
    ///        becomes `.metrics.csv`, otherwise `.metrics.csv` is appended. The
    ///        shared stem keeps the two files paired by run.
    static std::string metrics_path_for(const std::string& log_path);

    /// @brief Background thread body: drain both queues to disk and flush.
    void writer_loop();

    mutable std::mutex      m_mutex;
    std::condition_variable m_cv;
    std::deque<std::string> m_queue;         ///< Pending formatted log lines.
    std::deque<std::string> m_metrics_queue; ///< Pending raw metrics lines.
    bool                    m_stop;      ///< Set in the destructor to end the writer.
    std::ofstream           m_out;
    std::ofstream           m_metrics_out;
    std::string             m_path;
    bool                    m_open;
    bool                    m_metrics_open;  ///< Best-effort side-car; independent of m_open.
    std::thread             m_writer;    ///< Declared last; joined in the destructor.
};


#endif // LOGGER_H
