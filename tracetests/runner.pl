use strict;
use warnings;
use feature qw(say);
use Getopt::Long;



my %opts;
GetOptions(\%opts,
    'file|f=s',
) or die 'Invalid options found';

my %spawned_children; # name => pid


my %cmds = (
    spawn => \&spawn,
    spawn_with_log => \&spawn_with_log,
    stop => \&stop,
    sleep => sub { sleep shift; },
);

my $file = $opts{file};

my $read_handle;
if ( $file ) {
    open $read_handle, '<:encoding(utf8)', $file
        or die "Cannot open $file for reading";
} else {
    $read_handle = *STDIN;
}

my $test_name;

eval {
    while (<$read_handle>) {
        chomp;
        next if /^\s*#/; # comment line
        s/^\s*//;

        my ( $cmd, @args ) = split /\s+/;

        if ( !$test_name ) {
            die "first command should be the 'name' command"
                if $cmd ne 'name';
            $test_name = join ' ', @args;
            next;
        }


        if ( $cmd eq 'halt' ) {
            stop( keys %spawned_children );
            next;
        }

        if ( $cmd eq 'check' ) {
            stop( keys %spawned_children );
            my ( $checker, @script_args ) = @args;
            system $checker, @script_args;
            next;
        }

        die "Invalid command $cmd" unless exists $cmds{$cmd};

        $cmds{$cmd}->( @args );
    }
    1;
} or do {
    my $err = $@;
    stop( keys %spawned_children );
    die $err;
};

sub stop {
    my ( @names ) = @_;
    return unless @names;
    my @pids = delete @spawned_children{@names};
    _info( 'stopping pids: '.join(',', @pids) );
    my @signals = qw(TERM INT KILL);
    for my $pid ( @pids ) {
        say "Trying to stop pid $pid";
        my $killed = 0;
        for my $signal ( @signals ) {
            say "Trying SIG".$signal;
            $killed = kill $signal, $pid;
            if ( ! -e "/proc/${pid}" ) {
                $killed = 1;
                last;
            }
            sleep 1;
            if ( ! -e "/proc/${pid}" ) {
                $killed = 1;
                last;
            }
        }
        if ( !$killed or -e "/proc/${pid}" ) {
            say "Sending SIGTERM to the group";
            kill '-TERM', getpgrp($pid);
        }
    }
}

sub spawn {
    my ( $name, @cmdline ) = @_;
    return unless @cmdline;
    if ( my $pid = fork() ) {
        $spawned_children{$name} = $pid;
        _info( "$name started as pid $pid grp ".getpgrp($pid) );
    } else {
        my $cmd = join ' ', @cmdline;
        _info( "[child] exec '$cmd'" );
        exec $cmd;
    }
}

sub spawn_with_log {
    my ( $name, @cmdline ) = @_;
    my $logfile = sprintf '%s-%s.log', $test_name, $name;

    unlink $logfile or die "cannot remove $logfile: $!"
        if -e $logfile;

    push @cmdline, '2>&1', '|', 'tee', $logfile;
    spawn( $name, @cmdline );
}


sub _info {
    my $msg = shift;
    say '[runner] '.$msg;
}

# spawn name cmdline
# stop name
# sleep secs
# halt

