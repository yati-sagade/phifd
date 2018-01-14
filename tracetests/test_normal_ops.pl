use strict;
use warnings;
use feature qw(say);
use Getopt::Long;
use List::MoreUtils qw(all);
use Scalar::Util qw(looks_like_number);

use constant {
    NUM_PROCS => 3,
};

my %opts;
GetOptions(\%opts,
    'num_procs=i',
    'bind_host=s',
    'ping_interval=i',
    'test_runtime=i',
) or die 'Invalid options found';

my $num_procs = $opts{num_procs} || NUM_PROCS;

my $bind_host = $opts{bind_host} || '127.0.0.1';
die 'bind_host must name an ipv4 address'
    unless valid_ipv4_addr( $bind_host );

my $ping_interval = $opts{ping_interval} || 1; # seconds
die 'ping_interval must be a positive, integral number of seconds'
    unless $ping_interval > 0;

my $test_runtime = $opts{test_runtime} || 60; # seconds
die 'test_runtime must be a positive, integral number of seconds'
    unless $test_runtime > 0;

my @pids;
my $base_port = 12345;
my $start = time;
for my $procnum ( 1..NUM_PROCS ) {
    my $port = $base_port + $procnum - 1;
    my $pid = fork();
    if ( $pid ) {
        say "monitor: spawned $pid";
        push @pids, $pid;
        next;
    }
    my $bind_addr = $bind_host.':'.$port;
    my @cmd = ( '../target/debug/phifd',
                '-t', $ping_interval,
                '-a', $bind_addr );

    if ( $procnum > 1 ) {
        push @cmd, '-i', $bind_host.':'.$base_port; # introducer
    }

    my $logfile = sprintf '%s-%s.log', $bind_host, $port;

    unlink $logfile or die "Could not unlink $logfile: $!"
        if -e $logfile;

    push @cmd, '2>&1', '|', 'tee', $logfile;

    my $cmd = join ' ', @cmd;

    say "child: exec '$cmd', bye.";

    exec $cmd;
}

say "monitor: letting children run for ${test_runtime}s";

sleep $test_runtime;

say 'monitor: now terminating children';

kill 'TERM', @pids;

sub valid_ipv4_addr {
    my @octets = split /\./, $_[0];
    return all { looks_like_number($_) && $_ >= 0 && $_ <= 255 } @octets;
}
