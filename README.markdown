# phifd - Rust implementation of the Phi accrual failure detector

## Concepts

The [Phi accrual failure detector][1] enables applications to respond to
group members' suspicion levels, as opposed to binary {failed,alive} decisions
made by the failure detector itself.

For each member in the group, the FD outputs a suspicion value, guaranteed to
increase monotonically, and to tend to infinity for failed nodes. Applications
can contol the tradeoff between fast detection and high accuracy by tweaking
the threshold suspicion level `phi` at which a node is deemed failed.

A low value of `phi` will result in fast failure detections, at the expense of
increase false alarms, whereas increasing `phi` will make detections more
accurate, but will cause true detections of true failures to be delayed
accordingly.

One can hence build a group membership service on top of this FD by thresholding
appropriately the suspicion values for each node.

[1]: http://fubica.lsd.ufcg.edu.br/hp/cursos/cfsc/papers/hayashibara04theaccrual.pdf

