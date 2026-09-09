#include "gibson_settings.h"

#include <cctype>
#include <cerrno>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <pwd.h>
#include <sys/stat.h>
#include <unistd.h>
#include <vector>

static const GibsonSettings kDefaults = {
    .fly_speed = 0.55f,
    .bank_strength = 0.45f,
    .bank_max_degrees = 32.f,
    .bank_smoothing = 0.55f,
};
static const char *kUserRel = "Library/Application Support/TheGibson/config.txt";
static const char *kTemplate =
    "# The Gibson\n"
    "#\n"
    "# fly_speed — camera travel along the flythrough, in spline units per second.\n"
    "# The original demo was 0.9 (quite quick). Lower is slower.\n"
    "# Useful range: 0.3 (leisurely) to 0.9 (original).\n"
    "fly_speed = 0.55\n"
    "#\n"
    "# bank_strength — how hard the camera leans into turns (0 = level, 0.45 ≈ movie).\n"
    "# bank_max_degrees — clamp on roll. Negative bank_strength flips left/right.\n"
    "bank_strength = 0.45\n"
    "bank_max_degrees = 32\n"
    "# bank_smoothing — seconds of roll lag. Higher is silkier (0.3–0.8).\n"
    "bank_smoothing = 0.55\n";

static GibsonSettings g_settings = kDefaults;
static std::string g_path;
static bool g_loaded;

static std::string trim(std::string s)
{
    while (!s.empty() && std::isspace((unsigned char)s.front()))
        s.erase(s.begin());
    while (!s.empty() && std::isspace((unsigned char)s.back()))
        s.pop_back();
    return s;
}

static bool file_exists(const std::string &path)
{
    struct stat st;
    return stat(path.c_str(), &st) == 0 && S_ISREG(st.st_mode);
}

static std::string real_home()
{
    const struct passwd *pw = getpwuid(getuid());
    if (pw && pw->pw_dir && pw->pw_dir[0])
        return pw->pw_dir;
    const char *home = getenv("HOME");
    return (home && home[0]) ? std::string(home) : std::string();
}

static std::vector<std::string> user_config_paths()
{
    std::vector<std::string> paths;
    auto add = [&](const std::string &home) {
        if (home.empty())
            return;
        std::string path = home + "/" + kUserRel;
        for (const auto &existing : paths)
        {
            if (existing == path)
                return;
        }
        paths.push_back(path);
    };
    add(real_home());
    const char *envHome = getenv("HOME");
    if (envHome)
        add(envHome);
    return paths;
}

static bool parse_file(const std::string &path, GibsonSettings *out)
{
    FILE *fp = fopen(path.c_str(), "r");
    if (!fp)
        return false;

    char line[512];
    while (fgets(line, sizeof(line), fp))
    {
        std::string s = trim(line);
        if (s.empty() || s[0] == '#' || s[0] == ';')
            continue;
        const auto eq = s.find('=');
        if (eq == std::string::npos)
            continue;
        std::string key = trim(s.substr(0, eq));
        std::string val = trim(s.substr(eq + 1));
        if (key == "fly_speed" || key == "fly-speed" || key == "speed")
        {
            char *end = nullptr;
            float v = std::strtof(val.c_str(), &end);
            if (end != val.c_str())
            {
                if (v < 0.05f)
                    v = 0.05f;
                if (v > 3.f)
                    v = 3.f;
                out->fly_speed = v;
            }
        }
        else if (key == "bank_strength" || key == "bank-strength")
        {
            char *end = nullptr;
            float v = std::strtof(val.c_str(), &end);
            if (end != val.c_str())
            {
                if (v < -3.f)
                    v = -3.f;
                if (v > 3.f)
                    v = 3.f;
                out->bank_strength = v;
            }
        }
        else if (key == "bank_smoothing" || key == "bank-smoothing")
        {
            char *end = nullptr;
            float v = std::strtof(val.c_str(), &end);
            if (end != val.c_str())
            {
                if (v < 0.05f)
                    v = 0.05f;
                if (v > 2.f)
                    v = 2.f;
                out->bank_smoothing = v;
            }
        }
    }
    fclose(fp);
    return true;
}

static bool mkdir_p(const std::string &dir)
{
    if (dir.empty())
        return false;
    std::string cur;
    for (size_t i = 0; i < dir.size(); ++i)
    {
        cur.push_back(dir[i]);
        if (dir[i] == '/' && cur.size() > 1)
        {
            mkdir(cur.c_str(), 0755);
        }
    }
    return mkdir(dir.c_str(), 0755) == 0 || errno == EEXIST;
}

static void seed_user_config(const std::vector<std::string> &templates)
{
    std::string body = kTemplate;
    for (const auto &t : templates)
    {
        if (!file_exists(t))
            continue;
        FILE *in = fopen(t.c_str(), "r");
        if (!in)
            continue;
        std::string copied;
        char buf[1024];
        size_t n;
        while ((n = fread(buf, 1, sizeof(buf), in)) > 0)
            copied.append(buf, n);
        fclose(in);
        if (!copied.empty())
        {
            body.swap(copied);
            break;
        }
    }

    for (const auto &dest : user_config_paths())
    {
        if (dest.empty() || file_exists(dest))
            continue;
        const auto slash = dest.rfind('/');
        if (slash == std::string::npos)
            continue;
        mkdir_p(dest.substr(0, slash));
        FILE *out = fopen(dest.c_str(), "w");
        if (!out)
            continue;
        fwrite(body.data(), 1, body.size(), out);
        fclose(out);
    }
}

static void append_missing_keys(const std::string &path)
{
    if (!file_exists(path))
        return;
    FILE *in = fopen(path.c_str(), "r");
    if (!in)
        return;
    std::string body;
    char buf[1024];
    size_t n;
    while ((n = fread(buf, 1, sizeof(buf), in)) > 0)
        body.append(buf, n);
    fclose(in);
    if (body.find("bank_smoothing") != std::string::npos)
        return;
    FILE *out = fopen(path.c_str(), "a");
    if (!out)
        return;
    fputs("\n# bank_smoothing — seconds of roll lag. Higher is silkier.\n"
          "bank_smoothing = 0.55\n",
          out);
    fclose(out);
}

void gibson_settings_reload(const char *extra_dir)
{
    g_settings = kDefaults;
    g_path.clear();

    std::vector<std::string> candidates;
    for (const auto &user : user_config_paths())
        candidates.push_back(user);
    if (extra_dir && extra_dir[0])
    {
        std::string extra(extra_dir);
        while (!extra.empty() && extra.back() == '/')
            extra.pop_back();
        candidates.push_back(extra + "/config.txt");
    }
    candidates.push_back("config.txt");
    candidates.push_back("../config.txt");

    seed_user_config(candidates);
    for (const auto &user : user_config_paths())
        append_missing_keys(user);

    for (const auto &path : candidates)
    {
        GibsonSettings parsed = kDefaults;
        if (!file_exists(path) || !parse_file(path, &parsed))
            continue;
        g_settings = parsed;
        g_path = path;
        g_loaded = true;
        return;
    }
    g_loaded = true;
}

static void ensure_loaded()
{
    if (!g_loaded)
        gibson_settings_reload(nullptr);
}

GibsonSettings gibson_settings(void)
{
    ensure_loaded();
    return g_settings;
}

float gibson_fly_speed(void)
{
    ensure_loaded();
    return g_settings.fly_speed;
}

float gibson_bank_strength(void)
{
    ensure_loaded();
    return g_settings.bank_strength;
}

float gibson_bank_max_degrees(void)
{
    ensure_loaded();
    return g_settings.bank_max_degrees;
}

float gibson_bank_smoothing(void)
{
    ensure_loaded();
    return g_settings.bank_smoothing;
}

const char *gibson_settings_path(void)
{
    ensure_loaded();
    return g_path.empty() ? "" : g_path.c_str();
}
