#!/bin/sh

SCRIPTPATH=`realpath $0`
WORKDIR=`dirname $SCRIPTPATH`
cd $WORKDIR

SYSROOT=freebsd-sysroot
test -d $SYSROOT \
    || (mkdir $SYSROOT && cd $SYSROOT \
        && curl -LO http://ftp.cz.freebsd.org/pub/FreeBSD/releases/arm64/14.3-RELEASE/base.txz \
        && tar xvJf base.txz 2>/dev/null || true && rm base.txz)

make -B BUILD_BSD_INIT=1 SYSROOT_BSD=$SYSROOT -- init/init-freebsd
