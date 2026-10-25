#include <gtk/gtk.h>
#include <string.h>
#include <webkit2/webkit2.h>

/* A separate Linux app for the sync test. It is not Omacus.
   Id dev.omacus.OmacusTest. Pages stay in this app's own folders:
   ~/.local/share/omacus-test and ~/.cache/omacus-test.
   It does not open the installed app's pages, write its settings, or
   register a link handler. It does not write a desktop file. */

#define TEST_ID "dev.omacus.OmacusTest"
#define SYNC_URL "https://sync-test.omacus.com/"
#define DATA_NAME "omacus-test"
#define TEST_UA "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) omacus-test/1.0"

static gboolean owned_folder(const char *path) {
    const char *slash;
    const char *base;
    if (path == NULL || path[0] != '/')
        return FALSE;
    if (strstr(path, "/applications/") != NULL)
        return FALSE;
    slash = strrchr(path, '/');
    base = slash == NULL ? path : slash + 1;
    return strcmp(base, DATA_NAME) == 0;
}

static int prepare_store(char **data_out, char **cache_out) {
    const char *home = g_get_home_dir();
    char *data;
    char *cache;
    if (home == NULL || home[0] != '/') {
        fputs("omacus Test refused to open because it could not find a home folder.\n", stderr);
        return 1;
    }
    data = g_build_filename(home, ".local", "share", DATA_NAME, NULL);
    cache = g_build_filename(home, ".cache", DATA_NAME, NULL);
    if (!owned_folder(data) || !owned_folder(cache)) {
        fputs("omacus Test refused to open because its pages would be stored with the installed app.\n", stderr);
        g_free(data);
        g_free(cache);
        return 1;
    }
    if (g_mkdir_with_parents(data, 0700) != 0 || g_mkdir_with_parents(cache, 0700) != 0) {
        fputs("omacus Test could not create its own folders.\n", stderr);
        g_free(data);
        g_free(cache);
        return 1;
    }
    *data_out = data;
    *cache_out = cache;
    return 0;
}

static void activate(GtkApplication *app, gpointer user_data) {
    const char *id = g_application_get_application_id(G_APPLICATION(app));
    char *data = NULL;
    char *cache = NULL;
    WebKitWebsiteDataManager *manager;
    WebKitWebContext *context;
    const char *got_data;
    const char *got_cache;
    GtkWidget *window;
    GtkWidget *view;
    (void)user_data;
    if (id == NULL || strcmp(id, TEST_ID) != 0) {
        fprintf(stderr, "omacus Test refused to open because its id is %s.\n", id == NULL ? "" : id);
        g_application_quit(G_APPLICATION(app));
        return;
    }
    if (prepare_store(&data, &cache) != 0) {
        g_application_quit(G_APPLICATION(app));
        return;
    }
    manager = webkit_website_data_manager_new(
        "base-data-directory", data,
        "base-cache-directory", cache,
        NULL);
    got_data = webkit_website_data_manager_get_base_data_directory(manager);
    got_cache = webkit_website_data_manager_get_base_cache_directory(manager);
    if (!owned_folder(got_data) || !owned_folder(got_cache)) {
        fputs("omacus Test refused to open because its pages would be stored with the installed app.\n", stderr);
        g_object_unref(manager);
        g_free(data);
        g_free(cache);
        g_application_quit(G_APPLICATION(app));
        return;
    }
    context = webkit_web_context_new_with_website_data_manager(manager);
    window = gtk_application_window_new(app);
    gtk_window_set_title(GTK_WINDOW(window), "omacus Test");
    gtk_window_set_default_size(GTK_WINDOW(window), 1100, 760);
    view = webkit_web_view_new_with_context(context);
    webkit_settings_set_user_agent(webkit_web_view_get_settings(WEBKIT_WEB_VIEW(view)), TEST_UA);
    gtk_container_add(GTK_CONTAINER(window), view);
    webkit_web_view_load_uri(WEBKIT_WEB_VIEW(view), SYNC_URL);
    g_signal_connect_swapped(window, "destroy", G_CALLBACK(g_application_quit), app);
    gtk_widget_show_all(window);
    g_object_unref(context);
    g_object_unref(manager);
    g_free(data);
    g_free(cache);
}

int main(int argc, char **argv) {
    char *base;
    GtkApplication *app;
    int status;
    g_set_prgname("omacus-test");
    base = g_path_get_basename(argc > 0 ? argv[0] : "omacus-test");
    if (strcmp(base, "omacus") == 0) {
        fputs("omacus Test refused to open under the installed app's name.\n", stderr);
        g_free(base);
        return 1;
    }
    g_free(base);
    if (g_getenv("OMACUS_TEST_CHECK") != NULL) {
        char *data = NULL;
        char *cache = NULL;
        if (prepare_store(&data, &cache) != 0)
            return 1;
        printf("%s\n%s\n", data, cache);
        g_free(data);
        g_free(cache);
        return 0;
    }
    app = gtk_application_new(TEST_ID, G_APPLICATION_DEFAULT_FLAGS);
    g_signal_connect(app, "activate", G_CALLBACK(activate), NULL);
    status = g_application_run(G_APPLICATION(app), argc, argv);
    g_object_unref(app);
    return status;
}
