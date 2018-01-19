use v5.22;
use strict;
use warnings;
use feature qw(say);
use Getopt::Long;
use File::Spec;
use File::Path qw(make_path remove_tree);

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
my $logdir;
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
            $logdir = File::Spec->catfile( 'logs', $test_name );
            remove_tree( $logdir );
            make_path( $logdir );
            next;
        }


        if ( $cmd eq 'halt' ) {
            stop( keys %spawned_children );
            last;
        }

        if ( $cmd eq 'check' ) {
            my @children = keys %spawned_children;
            stop( @children );
            my ( $checker, @script_args ) = @args;
            _info( "Going to call checker script $checker now." );
            system $checker,
                    @script_args,
                    map File::Spec->catfile($logdir, $_.'.log'), @children;
            last;
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
    my $logfile = File::Spec->catfile( $logdir, $name.'.log' );
    push @cmdline, '2>&1', '|', 'tee', $logfile;
    spawn( $name, @cmdline );
}


sub _info {
    my $msg = shift;
    say '[runner] '.$msg;
}
