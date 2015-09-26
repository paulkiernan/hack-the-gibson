# The Gibson - A Makefile
#
# 'make'        build executable file 'gibson'
# 'make clean'  removes all .o and executable files
#

CC=g++

# compiler flags:
#   -g    adds debugging information to the executable file
#  -Wall turns on most, but not all, compiler warnings
CFLAGS=-g -Wall

INCLUDE_DIRS=\
	include \
	/usr/local/include \
	/usr/local/Cellar/irrlicht/1.8.1/include/irrlicht/  # You might have to adjust this for your machine
INCLUDES=$(foreach d, $(INCLUDE_DIRS), -I$d)

# define any libraries to link into executable:
#   if I want to link in libraries (libx.so or libx.a) I use the -llibname 
#   option, something like (this will link in libmylib.so and libm.so:
LIB_FILES=/usr/local/Cellar/irrlicht/1.8.1/lib/libIrrlicht.a
LIB_PATHS=/usr/local/lib
LIB_NAMES=\
	glfw3 \
	glew
LIBS=\
	$(foreach f, $(LIB_FILES), $f) \
	$(foreach d, $(LIB_PATHS), -L$d) \
	$(foreach n, $(LIB_NAMES), -l$n)


# define any frameworks we'll be using
FRAMEWORK_NAMES=\
	OpenGL \
	Cocoa \
	IOKit
LFLAGS=$(foreach n, $(FRAMEWORK_NAMES), -framework $n)

# define the c source files
SRCS=\
	main.cpp \
	PathCreator.cpp \
	pulse.cpp \
	room.cpp \
	cam.cpp \
	include/gui.cpp \
	include/camera.cpp \
	include/HardwareSkinCallback.cpp \
	include/events.cpp \
	include/globals.cpp \
	include/lighting.cpp \
	include/mesh.cpp \
	include/ray3d.cpp \
	include/render.cpp \
	include/Effects/CEffectPostProc.cpp \
	include/Effects/CLensFlarePostProc.cpp \
	include/Effects/CRendererPostProc.cpp \
	include/Effects/CSplitPostProc.cpp \
	include/Effects/CTransitionPostProc.cpp \
	include/Effects/CWaterPostProc.cpp \
	include/Effects/IPostProc.cpp

# define the C object files 
#
# This uses Suffix Replacement within a macro:
#   $(name:string1=string2)
#         For each word in 'name' replace 'string1' with 'string2'
# Below we are replacing the suffix .cpp of all words in the macro SRCS
# with the .o suffix
#
OBJS = $(SRCS:.cpp=.o)

MAIN=gibson

all: $(MAIN)
	@echo "Its time to hack the gibson."

$(MAIN): $(OBJS) 
	$(CC) $(CFLAGS) $(INCLUDES) -o $(MAIN) $(OBJS) $(LFLAGS) $(LIBS)

# this is a suffix replacement rule for building .o's from .c's
# it uses automatic variables $<: the name of the prerequisite of
# the rule(a .cpp file) and $@: the name of the target of the rule (a .o file) 
# (see the gnu make manual section about automatic variables)
.cpp.o:
	$(CC) $(CFLAGS) $(INCLUDES) -c $<  -o $@

clean:
	rm *.o $(MAIN)
	rm include/*.o
