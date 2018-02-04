package Local::PhiFD::TestNormalOps;

use strict;
use warnings;
use Data::Dumper;
use Getopt::Long;
use Pod::Usage;
use List::MoreUtils qw(all);
use Scalar::Util qw(looks_like_number);
use IPC::Run qw(run timeout start harness);
use Carp;
use File::Spec;
use File::Path qw(make_path remove_tree);
use File::Slurp qw(read_dir read_file write_file);
use JSON qw(encode_json);

use constant {
    TEST_NAME => 'test_normal_ops',
    PHI_MAX => 5,
};

sub new {
    my ( $class, %opts ) = @_;

    my $self = {};

    $self->{phifd} =
         $opts{phi_exec}
      || $ENV{PHIFD}
      || die 'Set PHIFD to the path to the FD binary.';

    $self->{test_runtime} = $opts{test_runtime} || 30;    # seconds
    die 'test_runtime must be a positive, integral number of seconds'
      unless $self->{test_runtime} > 0;

    $self->{ping_interval} = $opts{ping_interval} || 1;    # seconds
    die 'ping_interval must be a positive, integral number of seconds'
      unless $self->{ping_interval} > 0;

    $self->{log_root} = $opts{log_root} || 'logs/';
    $self->{test_name} = $opts{test_name};
    $self->{dump_phis} = $opts{dump_phis};
    $self->{num_nodes} = $opts{num_nodes} || 3;
    $self->{verbose}   = $opts{verbose};
    $self->{status}    = 'ok';

    bless $self, $class;

    return $self;
}

sub main {
    my ($self) = @_;

    local $SIG{__DIE__} = sub {
        my @args = @_;
        $self->cleanup;
        die @args;
    };

    $self->setup_logging();

    my $start_port = 12345;

    $self->log('going to spawn %d nodes', $self->{num_nodes});
    for my $procnum ( 0..$self->{num_nodes}-1 ) {
        my $port = $start_port + $procnum;
        my $cmd = [
            $self->{phifd}, '-t', $self->{ping_interval}, '-a',
            '127.0.0.1:'.$port
        ];

        # add introducer to all but the first command; the introducer node
        # is the one we spawned first.
        push @$cmd, '-i', "127.0.0.1:${start_port}" if $procnum > 0;

        my $logfile = File::Spec->catfile( $self->test_log_dir,
            'proc_' . $procnum . '.log' );

        # XXX: do we need an :encoding(utf8) discipline here?
        open my $fh, '>', $logfile
          or die "Cannot open $logfile for writing: $!";

        my $out = $self->{verbose} ? sub {
            my $fh = shift;
            sub {
                print $fh $_ for @_;
                print for @_;
            }
        }->($fh) : $fh;

        my $err = $out;

        my $harness = harness $cmd, \undef, $out, $err, init => sub {
            $ENV{RUST_BACKTRACE} = 1;
        };

        push @{ $self->{handles} },   $fh;
        push @{ $self->{harnesses} }, $harness;

        $self->log('spawned %s', join(' ', @$cmd));
    }

    $self->log( "started %d children", scalar( @{ $self->{harnesses} } ) );
    $self->log( "now waiting for %ds", $self->{test_runtime} );

    my $start   = time;
    my $elapsed = 0;
    my $logged;
    while ( $elapsed < $self->{test_runtime} ) {
        $elapsed = time - $start;
        $_->pump_nb for @{ $self->{harnesses} };
        if ( !defined $logged || ( $elapsed % 10 == 0 && $logged != $elapsed ) )
        {
            $logged = $elapsed;
            $self->log( '%ds to go', $self->{test_runtime} - $elapsed )
              if int($elapsed) % 10 == 0;
        }
    }

    $self->cleanup();
    $self->check();
    $self->post_check();

    return $self->{status} ne 'ok';
}

sub post_check {
    my ($self) = @_;
    if ( $self->{dump_phis} ) {
        if ( defined $self->{dump} ) {
            my $philename = $self->{test_name}.'-phis.json';
            $self->log("writing phi values to $philename.");
            write_file($philename, encode_json( $self->{dump} ));
        } else {
            $self->log('no stats to write out');
        }
    }
    $self->log( 'Test status: %s', $self->{status} );
}

sub cleanup {
    my ($self) = @_;
    $self->log( 'cleaning up %d children', scalar( @{ $self->{harnesses} } ) );
    $_->kill_kill for @{ $self->{harnesses} };
    @{ $self->{harnesses} } = ();
    close $_ for @{ $self->{handles} };
    @{ $self->{handles} } = ();
}

sub setup_logging {
    my ($self) = @_;
    remove_tree( $self->test_log_dir );
    make_path( $self->test_log_dir );
}

sub test_log_dir {
    my ($self) = @_;
    return File::Spec->catfile( $self->{log_root}, $self->{test_name} );
}

sub log {
    my ( $self, $fmt, @rest ) = @_;
    $fmt .= "\n" unless $fmt =~ /\n$/;
    printf "[runner] " . $fmt, @rest;
}

sub check {
    my ($self) = @_;
    my %files;
    for my $filename ( read_dir( $self->test_log_dir ) ) {
        my ($procnum) = $filename =~ /^proc_(\d+)\.log$/;
        next unless defined $procnum;
        $files{$procnum} = $filename;
    }
    $self->log( 'going to analyze %d files', 0 + keys(%files) );

=for comment

For each process, we'll keep another table keyed by process id,
essentially making a 2D table of stats:

|   x | 0                         | 1                         | 2                         |
|   0 | x                         | phi stats for 1 at node 0 | phi stats for 2 at node 0 |
|   1 | phi stats for 0 at node 1 | NA                        | phi stats for 2 at node 1 |
|   2 | phi stats for 0 at node 2 | phi stats for 1 at node 2 | NA                        |
| ... | ...                       | ...                       | ...                       |

=cut

    my %stats;
    for my $procnum ( keys %files ) {
        my $filename =
          File::Spec->catfile( $self->test_log_dir, $files{$procnum} );
        my @lines = read_file($filename);

        my $local_stats;
        my $our_key;
        for my $line (@lines) {
            if ( !defined $our_key ) {
                my ( $our_addr, $our_port ) = $line =~
                  /starting failure detector now on (\d+\.\d+\.\d+\.\d+):(\d+)/;
                $our_key = "$our_addr:$our_port"
                  if defined $our_addr and defined $our_port;
                next;
            }

            my ( $host, $port, $phi ) =
              $line =~ /phi\((\d+):(\d+)\)=\s*([^\s]+)/;
            next unless defined $host and defined $port and defined $phi;

            my $ip  = ip_number_to_dotted($host);
            my $key = "$ip:$port";

            $phi = lc $phi;    # in case it is Inf.
            die "funny value for phi matched: $phi"
              unless looks_like_number($phi)
              or $phi eq 'inf';

            $local_stats->{$key}{max} = $phi
              if !defined $local_stats->{$key}{max}
              || $local_stats->{$key}{max} < $phi;
            $local_stats->{$key}{min} = $phi
              if !defined $local_stats->{$key}{min}
              || $local_stats->{$key}{min} > $phi;
            $local_stats->{$key}{sum} += $phi;
            $local_stats->{$key}{sum_sq} += $phi * $phi;
            push @{ $local_stats->{$key}{phis} }, $phi;
        }

        die
          "could not determine our own address from the logs (process $procnum)"
          unless $our_key;

        if ( $self->{dump_phis} and defined $local_stats ) {
            for ( keys %$local_stats ) {
                my $phis = $local_stats->{$_}{phis};
                $self->{dump}{$our_key}{$_} = [@$phis];
            }
        }

        my %errors = $self->check_one($local_stats);
        if ( keys %errors ) {
            $self->log( 'errors at node %s: %s', $our_key, Dumper( \%errors ) );
            $self->{status} = 'fail';
        }
    }
}

sub graph {
    my ( $self, $host_id, $stats ) = @_;
    require Chart::Gnuplot;
    my @datasets;
    for my $node_id ( keys %$stats ) {
        my $node_stats = $stats->{$node_id};
        my $phis       = $node_stats->{phis};
        my @x          = ( 0 .. $#$phis );
        my $ds         = Chart::Gnuplot::DataSet->new(
            xdata  => \@x,
            ydata  => $phis,
            title  => "phis for $node_id",
            styles => 'linespoints',
        );
        push @datasets, $ds;
    }
    my $chart = Chart::Gnuplot->new(
        output => "${host_id}.png",
        title  => "phis at $host_id over time",
        xlabel => "iterations",
        ylabel => "phi",
    );
    $chart->plot2d(@datasets);
}

sub check_one {
    my ( $self, $stats ) = @_;
    my %errors;

    for my $node_id ( keys %$stats ) {
        my $node_stats = $stats->{$node_id};
        my $phis       = $node_stats->{phis};
        my $prev       = $phis->[4];
        my @too_high_indexes; # Indexes into @$phis where phi values are too high.
        for my $idx ( 0 .. $#$phis ) {
            my $phi = $phis->[$idx];
            if ( $phi > PHI_MAX ) {
                push @too_high_indexes, $idx;
            }
        }
        if ( @too_high_indexes ) {
            my $msg = 'High values of phi encountered: '.
                join(',', map "[$_]:".$phis->[$_], @too_high_indexes);
            $errors{$node_id} = $msg;
        }
    }
    return %errors;
}

sub ip_number_to_dotted {
    my $num = shift;
    return sprintf '%d.%d.%d.%d',
      ( $num >> 24 ) & 0xff,
      ( $num >> 16 ) & 0xff,
      ( $num >> 8 ) & 0xff,
      $num & 0xff,
      ;
}

do {

    my %opts;
    GetOptions( \%opts, 'ping_interval=i', 'test_runtime=i', 'phi_exec=s',
        'log_root_dir=s', 'dump_phis', 'help', 'num_nodes=i', 'verbose' )
      or pod2usage(2);

    pod2usage(1) if delete $opts{help};

    exit( __PACKAGE__->new( %opts, test_name => TEST_NAME )->main() );

} unless (caller);

1;

__END__

=pod

=head1 OPTIONS

=over

=item --help

Show this message and quit

=item --ping_interval=INTERVAL

How often to ping peers (passed through to the failure detector)

=item --test_runtime=SECS

How long to collect the trace for

=item --phi_exec=PATH

Path to the phifd binary. When not given the PHIFD environment variable is
consulted, and if that is not set an error is raised.

=item --log_root_dir

Path to the root directory for collecting test logs. This test creates a
subdirectory called C<test_normal_ops> under the root. The process logs are
found inside this test subdir.

=item --dump_phis

Dump the statistics scraped from the logs about phi values into C<phis.json>.
The top level keys of the encoded object represent nodes, and the second level
keys are the other nodes in the cluster that the toplevel keys peer with.

=item --num_nodes=N

Number of nodes to spawn

=item --verbose

Print the stdout of the processes to STDOUT as well.

=cut
