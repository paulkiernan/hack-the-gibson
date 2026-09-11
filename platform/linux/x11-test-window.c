/* x11-test-window.c --- a throwaway X11 client for the xscreensaver smoke test.
 *
 * Creates and maps a window, prints its id on stdout as 0x<hex>, then waits.
 * On SIGTERM or SIGINT, or after --seconds, it destroys the window and exits 0
 * --- which is what xscreensaver does to a hack when the saver stops, and what
 * the hack has to notice and exit cleanly on.
 *
 * This stands in for xscreensaver itself, because the hack only ever sees the
 * window id: the daemon runs the `programs:` line and passes the window in
 * $XSCREENSAVER_WINDOW, and its settings dialog appends --window-id <id> to
 * the command line. Both are reproduced by platform/linux/smoke-test.sh.
 *
 * Built on the fly by that script; not part of any shipped artifact.
 *
 * Usage: x11-test-window [--size WxH] [--seconds N]
 *        stdout: the window id, e.g. 0x400005
 *        stderr: what was created, for the CI log
 *
 * Exit status: 0 on success, 2 if the display cannot be opened, 3 if the
 * window could not be mapped (a window that is not viewable cannot be drawn
 * into, and the failure mode is otherwise an opaque error from the renderer).
 */

#include <X11/Xlib.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static Display *dpy;
static Window win;

/* Set from the signal handler, checked from the sleep loop: destroying the
   window from inside a handler would call into Xlib, which is not
   async-signal-safe. */
static volatile sig_atomic_t stopping = 0;

static void
stop (int sig)
{
  (void) sig;
  stopping = 1;
}

static int
parse_size (const char *s, unsigned int *w, unsigned int *h)
{
  char trailing;
  return (2 == sscanf (s, "%ux%u%c", w, h, &trailing) && *w > 0 && *h > 0);
}

int
main (int argc, char **argv)
{
  unsigned int width = 640, height = 480;
  int seconds = 20;
  int i;

  for (i = 1; i < argc; i++)
    {
      if (!strcmp (argv[i], "--size") && i + 1 < argc)
        {
          if (!parse_size (argv[++i], &width, &height))
            {
              fprintf (stderr, "x11-test-window: bad --size %s\n", argv[i]);
              return 2;
            }
        }
      else if (!strcmp (argv[i], "--seconds") && i + 1 < argc)
        {
          seconds = atoi (argv[++i]);
          if (seconds <= 0)
            {
              fprintf (stderr, "x11-test-window: bad --seconds %s\n", argv[i]);
              return 2;
            }
        }
      else
        {
          fprintf (stderr, "x11-test-window: unknown argument %s\n", argv[i]);
          return 2;
        }
    }

  signal (SIGTERM, stop);
  signal (SIGINT, stop);

  dpy = XOpenDisplay (NULL);
  if (!dpy)
    {
      fprintf (stderr, "x11-test-window: cannot open display %s\n",
               getenv ("DISPLAY") ? getenv ("DISPLAY") : "(DISPLAY unset)");
      return 2;
    }

  {
    int screen = DefaultScreen (dpy);
    win = XCreateSimpleWindow (dpy, RootWindow (dpy, screen), 0, 0,
                               width, height, 0,
                               BlackPixel (dpy, screen),
                               BlackPixel (dpy, screen));
    XMapWindow (dpy, win);
    XSync (dpy, False);
  }

  {
    XWindowAttributes attrs;
    if (!XGetWindowAttributes (dpy, win, &attrs))
      {
        fprintf (stderr, "x11-test-window: cannot query window 0x%lx\n",
                 (unsigned long) win);
        return 3;
      }
    fprintf (stderr, "x11-test-window: window 0x%lx %ux%u depth %d visual 0x%lx\n",
             (unsigned long) win, attrs.width, attrs.height, attrs.depth,
             attrs.visual ? (unsigned long) XVisualIDFromVisual (attrs.visual) : 0UL);
    if (attrs.map_state != IsViewable)
      {
        fprintf (stderr, "x11-test-window: window 0x%lx is not viewable after "
                 "XMapWindow (map_state %d); the hack cannot draw into it\n",
                 (unsigned long) win, (int) attrs.map_state);
        return 3;
      }
  }

  printf ("0x%lx\n", (unsigned long) win);
  fflush (stdout);

  /* Sleep in one-second slices: sleep() returns early when a signal arrives,
     so a SIGTERM is noticed within a second and falls through to the destroy
     below, exactly like xscreensaver tearing the hack's window down. */
  for (i = 0; i < seconds && !stopping; i++)
    sleep (1);

  XDestroyWindow (dpy, win);
  XSync (dpy, False);
  XCloseDisplay (dpy);
  return 0;
}
